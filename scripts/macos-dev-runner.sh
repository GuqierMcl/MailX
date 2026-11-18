#!/bin/sh
# Keep the native app at the runner PID so Tauri watch can terminate it directly.
set -eu

runtime=$1
signer=$2
binary=$3
shift 3

"$runtime" "$signer" --sign-development "$binary"
exec "$binary" "$@"
