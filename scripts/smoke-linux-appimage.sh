#!/usr/bin/env bash
# Build a clean Ubuntu 22.04 runtime, then check the AppImage with and without
# its bundled AppIndicator. Neither scenario has host tray packages or network.
set -euo pipefail

script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)

fail() {
  echo "$*" >&2
  exit 1
}

session_check() {
  local scenario=$1 launcher=$2 appdir=$3 data_dir=$4 evidence=$5
  local app_pid window_id='' window_pid='' attempt
  mkdir -p "$evidence"
  [[ ! -e $data_dir ]] || fail "The smoke profile must be new: $data_dir"
  export MAILX_DATA_DIR="$data_dir"
  export APPIMAGE_EXTRACT_AND_RUN=1
  # Xvfb has no GPU. Match the AppImageHub test's software rendering settings;
  # these are rendering switches and leave WebKit's sandbox enabled.
  export WEBKIT_DISABLE_DMABUF_RENDERER=1
  export WEBKIT_DISABLE_COMPOSITING_MODE=1
  if [[ $scenario == without-tray ]]; then
    export APPDIR="$appdir"
  fi
  # A separate process group lets cleanup include WebKit's subprocesses.
  setsid "$launcher" > "$evidence/app.log" 2>&1 &
  app_pid=$!
  # shellcheck disable=SC2064 # Capture the PID now for this session's cleanup.
  trap "kill -TERM -- -$app_pid 2>/dev/null || true; sleep 1; kill -KILL -- -$app_pid 2>/dev/null || true; wait $app_pid 2>/dev/null || true" EXIT
  for ((attempt=0; attempt<30; attempt++)); do
    kill -0 "$app_pid" 2>/dev/null || {
      cat "$evidence/app.log" >&2
      fail "$scenario: AppImage exited before showing its main window."
    }
    window_id=$(xdotool search --onlyvisible --name '^MailX$' 2>/dev/null | head -n 1 || true)
    [[ -z $window_id ]] || break
    sleep 1
  done
  [[ -n $window_id ]] || {
    cat "$evidence/app.log" >&2
    fail "$scenario: no visible MailX window within 30 seconds."
  }
  window_pid=$(xdotool getwindowpid "$window_id")
  tr '\0' '\n' < "/proc/$window_pid/environ" \
    | grep -E '^(MAILX_DATA_DIR|WEBKIT_[A-Z_]+)=' > "$evidence/process-environment.txt"
  if grep -q '^WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS=' "$evidence/process-environment.txt"; then
    fail "$scenario: the application disabled WebKit's sandbox."
  fi
  xwininfo -id "$window_id" > "$evidence/window.txt"
  # Observe startup long enough to catch the former tray-initialization panic.
  for ((attempt=0; attempt<10; attempt++)); do
    sleep 1
    if ! kill -0 "$app_pid" 2>/dev/null || ! kill -0 "$window_pid" 2>/dev/null; then
      cat "$evidence/app.log" >&2
      fail "$scenario: application exited during the startup observation."
    fi
    xdotool search --onlyvisible --name '^MailX$' > /dev/null || fail "$scenario: the main window disappeared."
  done
  ps -o pid,ppid,user,args -p "$app_pid,$window_pid" > "$evidence/process.txt"
  local fallback_marker='AppIndicator runtime unavailable; Linux tray and background hiding disabled.'
  if [[ $scenario == without-tray ]]; then
    grep -Fq "$fallback_marker" "$evidence/app.log" || fail "$scenario: the missing-library fallback was not exercised."
  elif grep -Fq "$fallback_marker" "$evidence/app.log"; then
    fail "$scenario: bundled AppIndicator did not load during normal startup."
  fi
  python3 - "$data_dir" > "$evidence/profile.json" <<'PY'
import json
import pathlib
import sqlite3
import sys

profile = pathlib.Path(sys.argv[1])
database = profile / "mailx.sqlite3"
if not database.is_file():
    raise SystemExit("Startup did not create the isolated SQLite database")
with sqlite3.connect(database.as_uri() + "?mode=ro", uri=True) as db:
    integrity = db.execute("PRAGMA integrity_check").fetchone()[0]
    version = db.execute("PRAGMA user_version").fetchone()[0]
    migrations = db.execute("SELECT COUNT(*), MAX(version) FROM schema_migrations").fetchone()
    accounts = db.execute("SELECT COUNT(*) FROM accounts").fetchone()[0]
    if integrity != "ok" or version < 1 or migrations[1] != version or accounts != 0:
        raise SystemExit("Unexpected schema or accounts in the empty smoke profile")
    print(json.dumps({"profile": str(profile), "database": str(database),
                      "integrity": integrity, "user_version": version,
                      "migration_count": migrations[0], "accounts": accounts}, indent=2))
PY
  printf '%s: visible main window and live process; isolated empty profile initialized.\n' "$scenario"
}

container_check() {
  local appimage=$1 evidence=$2 work_dir appdir runtime_dir
  [[ $(id -u) -ne 0 ]] || fail "Run the application as a non-root user."
  mkdir -p "$evidence"
  {
    cat /etc/os-release
    id
    printf '\nNetwork interfaces (Docker is launched with --network none):\n'
    cat /proc/net/dev
    printf '\nWebKit sandbox environment overrides must be absent:\n'
    env | sort | grep -E 'WEBKIT.*SANDBOX' || true
  } > "$evidence/runtime.txt"
  [[ -z ${WEBKIT_DISABLE_SANDBOX_THIS_IS_DANGEROUS:-} && -z ${WEBKIT_FORCE_SANDBOX:-} ]] || fail "WebKit sandbox overrides are not permitted."
  ldconfig -p > "$evidence/system-libraries.txt"
  if grep -Eq 'lib(ayatana-appindicator|appindicator|ayatana-indicator|ayatana-ido|dbusmenu)' "$evidence/system-libraries.txt"; then
    fail "The clean runtime unexpectedly contains a system tray dependency."
  fi
  work_dir=$(mktemp -d)
  # All data stays in a temporary synthetic profile, separate from the evidence.
  # shellcheck disable=SC2064 # Capture the temporary directory before returning.
  trap "rm -rf -- '$work_dir'" EXIT
  runtime_dir="$work_dir/runtime"
  mkdir -m 700 "$runtime_dir"
  export XDG_RUNTIME_DIR="$runtime_dir"
  (cd "$work_dir" && "$appimage" --appimage-extract > "$evidence/extract.log")
  appdir="$work_dir/squashfs-root"
  [[ -x $appdir/AppRun ]] || fail "AppImage extraction did not produce AppRun."
  bash "$script_dir/check-linux-appimage.sh" "$appdir" "$evidence/dependencies" "$appimage"
  dbus-run-session -- xvfb-run -a -s '-screen 0 1280x1024x24' \
    bash "$script_dir/smoke-linux-appimage.sh" --session normal "$appimage" "$appdir" "$work_dir/normal-profile" "$evidence/normal"
  # Remove every alternate name as well as the exact SONAME so a symlink cannot
  # accidentally make the fallback case load the original library.
  find "$appdir" \( -type f -o -type l \) \
    \( -name 'libayatana-appindicator3.so*' -o -name 'libappindicator3.so*' \) \
    -print -delete > "$evidence/removed-tray-libraries.txt"
  [[ -s $evidence/removed-tray-libraries.txt ]] || fail "The fallback scenario did not remove a tray library."
  dbus-run-session -- xvfb-run -a -s '-screen 0 1280x1024x24' \
    bash "$script_dir/smoke-linux-appimage.sh" --session without-tray "$appdir/AppRun" "$appdir" "$work_dir/without-tray-profile" "$evidence/without-tray"
  echo "Both Linux AppImage startup scenarios passed." > "$evidence/result.txt"
}

case ${1:-} in
  --help)
    echo "Usage: bash scripts/smoke-linux-appimage.sh APPIMAGE [EVIDENCE_DIR]"
    echo "Requires Linux and Docker; evidence defaults to target/linux-appimage-evidence."
    exit 0
    ;;
  --session)
    shift
    [[ $# -eq 5 ]] || fail "Invalid internal session arguments."
    session_check "$@"
    exit 0
    ;;
  --container)
    shift
    [[ $# -eq 2 ]] || fail "Invalid internal container arguments."
    container_check "$@"
    exit 0
    ;;
esac

[[ $# -ge 1 && $# -le 2 && $(uname -s) == Linux ]] || fail "Expected an AppImage path on Linux; see --help."
command -v docker > /dev/null || fail "Docker is required for the clean runtime smoke test."
[[ $(id -u) -ne 0 ]] || fail "Use a non-root host user so the evidence has the same owner."
appimage=$(realpath "$1")
[[ -x $appimage ]] || fail "AppImage must exist and be executable: $appimage"
mkdir -p "${2:-target/linux-appimage-evidence}"
evidence=$(realpath "${2:-target/linux-appimage-evidence}")
image_name="mailx-appimage-smoke:$(id -u)"

cat > "$evidence/Dockerfile" <<'DOCKERFILE'
FROM ubuntu:22.04
ARG TEST_UID
ARG TEST_GID
RUN apt-get update && apt-get install -y --no-install-recommends \
      dbus-x11 xvfb xauth xdotool x11-utils binutils python3 procps util-linux \
      libwebkit2gtk-4.1-0 libxdo3 libssl3 librsvg2-2 libasound2 fonts-dejavu-core \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid "$TEST_GID" mailx-smoke \
    && useradd --uid "$TEST_UID" --gid "$TEST_GID" --create-home mailx-smoke
USER mailx-smoke
WORKDIR /tmp
DOCKERFILE
docker build --build-arg "TEST_UID=$(id -u)" --build-arg "TEST_GID=$(id -g)" \
  --tag "$image_name" - < "$evidence/Dockerfile" 2>&1 | tee "$evidence/container-build.log"
docker image inspect "$image_name" > "$evidence/container-image.json"
# WebKit's own bubblewrap sandbox needs user/mount namespaces that Docker's
# default seccomp and AppArmor profiles block. Relax those outer profiles only;
# keep WebKit's sandbox enabled, drop capabilities, and provide no network.
docker run --rm --network none --cap-drop ALL \
  --security-opt seccomp=unconfined --security-opt apparmor=unconfined \
  --mount "type=bind,src=$appimage,dst=/input/MailX.AppImage,readonly" \
  --mount "type=bind,src=$script_dir,dst=/checks,readonly" \
  --mount "type=bind,src=$evidence,dst=/evidence" \
  "$image_name" bash /checks/smoke-linux-appimage.sh --container /input/MailX.AppImage /evidence \
  2>&1 | tee "$evidence/runtime.log"
