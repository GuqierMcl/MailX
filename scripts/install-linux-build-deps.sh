#!/usr/bin/env bash
set -euo pipefail

packages=(
  libwebkit2gtk-4.1-dev libxdo-dev libssl-dev libayatana-appindicator3-dev
  librsvg2-dev libdbus-1-dev libasound2-dev pkg-config
)
case "${1:-}" in
  --packaging) packages+=(patchelf xdg-utils rpm rpm2cpio cpio) ;;
  '') ;;
  *) echo 'Usage: bash scripts/install-linux-build-deps.sh [--packaging]' >&2; exit 2 ;;
esac

sources=(/etc/apt/sources.list /etc/apt/sources.list.d/ubuntu.sources /etc/apt/apt-mirrors.txt)
for source in "${sources[@]}"; do
  if [ -f "$source" ]; then sudo cp -a "$source" "$source.runner"; fi
done

use_runner_mirrors() {
  for source in "${sources[@]}"; do
    if [ -f "$source.runner" ]; then sudo cp -a "$source.runner" "$source"; fi
  done
}

# Support both direct sources and GitHub's mirror+file layout. APT verifies the
# archive signatures regardless of which mirror served the files.
use_mirrors() {
  use_runner_mirrors
  for source in /etc/apt/sources.list /etc/apt/sources.list.d/ubuntu.sources; do
    if [ -f "$source" ]; then
      sudo sed -i -E 's#https?://('"$1"')[.]ubuntu[.]com/'"$2"'(/|[[:space:]]|$)#'"$3"'\2#g' "$source"
    fi
  done
  if [ -f /etc/apt/apt-mirrors.txt ]; then
    printf '%s/\tpriority:1\n' "$3" | sudo tee /etc/apt/apt-mirrors.txt > /dev/null
    if [ "$#" -eq 4 ]; then
      printf '%s/\tpriority:2\n' "$4" | sudo tee -a /etc/apt/apt-mirrors.txt > /dev/null
    fi
  fi
}

apt_options=(-o Acquire::Retries=3 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30)
install_dependencies() {
  sudo timeout 180 apt-get "${apt_options[@]}" update --error-on=any &&
    sudo timeout 300 apt-get "${apt_options[@]}" install -y --no-install-recommends "${packages[@]}"
}

case "$(dpkg --print-architecture)" in
  amd64)
    use_mirrors 'azure[.]archive' ubuntu https://archive.ubuntu.com/ubuntu https://security.ubuntu.com/ubuntu
    fallback=( 'azure[.]archive|archive|security' ubuntu https://mirrors.edge.kernel.org/ubuntu https://mirrors.ocf.berkeley.edu/ubuntu )
    ;;
  arm64)
    use_mirrors '[[:alnum:].-]+' ubuntu-ports https://ports.ubuntu.com/ubuntu-ports
    fallback=( '[[:alnum:].-]+' ubuntu-ports https://mirrors.ocf.berkeley.edu/ubuntu-ports )
    ;;
  *) echo 'Expected an Ubuntu amd64 or arm64 runner.' >&2; exit 2 ;;
esac

if install_dependencies; then exit 0; fi
echo '::warning::Official Ubuntu mirrors failed; retrying with the runner mirror.'
use_runner_mirrors
if install_dependencies; then exit 0; fi
echo '::warning::The runner mirror failed; retrying with community mirrors.'
use_mirrors "${fallback[@]}"
install_dependencies
