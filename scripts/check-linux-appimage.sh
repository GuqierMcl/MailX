#!/usr/bin/env bash
# Check the dlopen-only tray dependency chain inside an extracted AppImage.
# Run this in the same clean Linux runtime used by smoke-linux-appimage.sh.
set -euo pipefail

if [[ ${1:-} == --help ]]; then
  echo "Usage: bash scripts/check-linux-appimage.sh APPDIR EVIDENCE_DIR [APPIMAGE]"
  exit 0
fi
if [[ $# -lt 2 || $# -gt 3 || $(uname -s) != Linux ]]; then
  echo "Expected an extracted AppDir and evidence directory on Linux." >&2
  exit 2
fi

appdir=$(realpath "$1")
mkdir -p "$2"
evidence=$(realpath "$2")
libraries=(
  libayatana-appindicator3.so.1
  libayatana-indicator3.so.7
  libayatana-ido3-0.4.so.0
  libdbusmenu-glib.so.4
  libdbusmenu-gtk3.so.4
)
case $(uname -m) in
  x86_64) elf_machine='Advanced Micro Devices X86-64'; multiarch=x86_64-linux-gnu ;;
  aarch64) elf_machine=AArch64; multiarch=aarch64-linux-gnu ;;
  *) echo "Unsupported AppImage smoke host architecture: $(uname -m)" >&2; exit 2 ;;
esac
library_path="$appdir/usr/lib:$appdir/usr/lib/$multiarch:$appdir/lib:$appdir/lib/$multiarch"

# Inspect GNU version requirements, not strings or version definitions (which
# advertise what a library provides rather than what it needs from glibc).
python3 - "$appdir" "$elf_machine" "${3:-}" > "$evidence/glibc.json" <<'PY'
import json
import pathlib
import re
import subprocess
import sys

appdir = pathlib.Path(sys.argv[1])
expected_machine = sys.argv[2]
files = [(path, str(path.relative_to(appdir))) for path in appdir.rglob("*")
         if path.is_file() and not path.is_symlink()]
if sys.argv[3]:
    files.append((pathlib.Path(sys.argv[3]), "@AppImage-runtime"))
records = []
errors = []
limit = (2, 39)
highest = None
for path, name in sorted(files, key=lambda item: item[1]):
    with path.open("rb") as stream:
        if stream.read(4) != b"\x7fELF":
            continue
    result = subprocess.run(["readelf", "--file-header", "--version-info", "--wide", str(path)],
                            capture_output=True, text=True, env={"LC_ALL": "C"})
    if result.returncode:
        errors.append({"file": name, "error": result.stderr})
        continue
    in_needs = False
    has_needs = False
    machine = None
    versions = set()
    for line in result.stdout.splitlines():
        if line.lstrip().startswith("Machine:"):
            machine = line.split(":", 1)[1].strip()
        if line.startswith("Version "):
            in_needs = line.startswith("Version needs section")
            has_needs |= in_needs
        if in_needs:
            versions.update(re.findall(r"\bName: (GLIBC_[A-Za-z0-9_.]+)", line))
    if machine != expected_machine:
        errors.append({"file": name, "error": f"Expected {expected_machine} ELF, got {machine}"})
    dynamic = subprocess.run(["readelf", "--dynamic", "--wide", str(path)],
                             capture_output=True, text=True, env={"LC_ALL": "C"})
    if dynamic.returncode or ("(VERNEED)" in dynamic.stdout and not has_needs):
        errors.append({"file": name, "error": "Cannot inspect ELF version-needs table"})
    for version in versions:
        suffix = version.removeprefix("GLIBC_")
        if not re.fullmatch(r"[0-9]+(?:\.[0-9]+)+", suffix):
            errors.append({"file": name, "error": "Unknown glibc requirement: " + version})
            continue
        numeric = tuple(map(int, suffix.split(".")))
        if highest is None or numeric > highest:
            highest = numeric
        if numeric > limit:
            errors.append({"file": name, "error": "Above Ubuntu 24.04 baseline: " + version})
    records.append({"file": name, "machine": machine, "required_versions": sorted(versions)})
if not records:
    errors.append({"error": "No ELF files were checked"})
print(json.dumps({"ok": not errors, "maximum_allowed": "GLIBC_2.39",
                  "highest_required": "GLIBC_" + ".".join(map(str, highest)) if highest else None,
                  "elf_files_checked": len(records), "errors": errors,
                  "files": records}, indent=2))
if errors:
    raise SystemExit("AppImage ELF architecture or glibc verification failed; see glibc.json")
PY

for name in "${libraries[@]}"; do
  library="$appdir/usr/lib/$name"
  if [[ ! -f $library ]]; then
    echo "AppImage is missing its bundled tray dependency: $name" >&2
    exit 1
  fi
  readelf --file-header "$library" > "$evidence/$name.elf.txt"
  readelf --dynamic "$library" > "$evidence/$name.dynamic.txt"
  soname=$(awk -F '[][]' '/\(SONAME\)/ { print $2 }' "$evidence/$name.dynamic.txt")
  if [[ $soname != "$name" ]]; then
    echo "Unexpected SONAME for $library: $soname" >&2
    exit 1
  fi
  if ! LD_LIBRARY_PATH="$library_path" ldd "$library" > "$evidence/$name.ldd.txt" 2>&1; then
    cat "$evidence/$name.ldd.txt" >&2
    exit 1
  fi
  if grep -q 'not found' "$evidence/$name.ldd.txt"; then
    cat "$evidence/$name.ldd.txt" >&2
    exit 1
  fi
done

# The dependency is loaded dynamically by libappindicator-sys, so inspecting
# only the application's DT_NEEDED entries cannot detect this regression.
LD_LIBRARY_PATH="$library_path" python3 - "$appdir" > "$evidence/tray-load.json" <<'PY'
import ctypes
import json
import pathlib
import sys

library = pathlib.Path(sys.argv[1]) / "usr/lib/libayatana-appindicator3.so.1"
loaded = ctypes.CDLL(str(library))
print(json.dumps({"library": loaded._name, "loaded": True}, indent=2))
PY
echo "Bundled AppIndicator and its dependency chain loaded successfully."
