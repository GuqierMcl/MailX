#!/usr/bin/env bash
# Verify the package architecture and both executables in each Linux installer.
set -euo pipefail

fail() {
  echo "$*" >&2
  exit 1
}

[[ $# -eq 4 ]] || fail "Usage: bash scripts/check-linux-installers.sh PLATFORM DEB RPM APPIMAGE"
case $1 in
  linux-x64)
    deb_arch=amd64
    rpm_arch=x86_64
    elf_machine='Advanced Micro Devices X86-64'
    ;;
  linux-arm64)
    deb_arch=arm64
    rpm_arch=aarch64
    elf_machine=AArch64
    ;;
  *) fail "Unknown Linux platform: $1" ;;
esac

deb=$(realpath "$2")
rpm=$(realpath "$3")
appimage=$(realpath "$4")
scratch=$(mktemp -d)
trap 'rm -rf -- "$scratch"' EXIT

require_arch() {
  local path=$1 expected=$2 actual=$3
  [[ $actual == "$expected" ]] || fail "$path has architecture '$actual', expected '$expected'."
}

check_elf() {
  local binary=$1
  [[ -x $binary ]] || fail "Missing or non-executable binary: $binary"
  local header
  header=$(LC_ALL=C readelf --file-header --wide "$binary")
  require_arch "$binary" "$elf_machine" "$(awk -F ': *' '/Machine:/ { print $2 }' <<< "$header")"
}

require_arch "$deb" "$deb_arch" "$(dpkg-deb -f "$deb" Architecture)"
require_arch "$rpm" "$rpm_arch" "$(rpm -qp --qf '%{ARCH}' "$rpm")"
dpkg-deb -x "$deb" "$scratch/deb"
mkdir "$scratch/rpm"
rpm2cpio "$rpm" | (cd "$scratch/rpm" && cpio --extract --make-directories --no-absolute-filenames --quiet)

check_elf "$appimage"
(cd "$scratch" && "$appimage" --appimage-extract > /dev/null)
for root in "$scratch/deb" "$scratch/rpm" "$scratch/squashfs-root"; do
  check_elf "$root/usr/bin/mailx-desktop"
  check_elf "$root/usr/bin/mailx-mcp"
done

echo "Verified DEB, RPM and AppImage executables for $1."
