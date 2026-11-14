#!/usr/bin/env bash
# Intel targets below macOS 10.14 record their SDK in LC_VERSION_MIN_MACOSX;
# newer targets use LC_BUILD_VERSION. Both describe the SDK, not just the OS.
set -euo pipefail

if [[ $# -ne 1 || ! -d $1/Contents/MacOS ]]; then
  echo 'Usage: bash scripts/check-macos-sdk.sh APP_BUNDLE' >&2
  exit 2
fi

checked=0
for binary in "$1"/Contents/MacOS/*; do
  [[ -f $binary && -x $binary ]] || continue
  versions=$(otool -l "$binary" | awk '
    $1 == "cmd" { version_command = ($2 == "LC_BUILD_VERSION" || $2 == "LC_VERSION_MIN_MACOSX") }
    version_command && $1 == "sdk" { print $2 }
  ')
  if [[ -z $versions ]]; then
    echo "::error::$binary has no macOS SDK metadata." >&2
    exit 1
  fi
  while IFS= read -r sdk; do
    echo "$binary: SDK $sdk"
    if [[ ! $sdk =~ ^[0-9]+(\.[0-9]+)*$ ]] || ! [ "${sdk%%.*}" -ge 26 ] 2>/dev/null; then
      echo "::error::$binary was built with macOS SDK $sdk; SDK 26 or later is required." >&2
      exit 1
    fi
  done <<< "$versions"
  checked=$((checked + 1))
done
if [[ $checked -eq 0 ]]; then
  echo '::error::The app bundle contains no executable to check.' >&2
  exit 1
fi
