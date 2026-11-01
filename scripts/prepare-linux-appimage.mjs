// Make Tauri's AppImage contain libraries loaded by libappindicator-sys via
// dlopen. They do not appear in the executable's DT_NEEDED entries, so the
// normal linuxdeploy dependency scan cannot discover them from the executable.
//
// Run on the Linux build host after installing libayatana-appindicator3-dev:
//   bun scripts/prepare-linux-appimage.mjs --out /tmp/tauri-linux-appimage.json
//   bun run tauri build --config /tmp/tauri-linux-appimage.json
//
// Tauri copies appimage.files before linuxdeploy scans existing AppDir ELFs.
// The five explicit SONAMEs cover Ayatana and its Indicator/Ido/dbusmenu chain;
// linuxdeploy then handles their ordinary GTK/GLib dependencies and RPATHs.
import { spawnSync } from "node:child_process";
import { closeSync, mkdirSync, openSync, readSync, realpathSync, statSync, writeFileSync } from "node:fs";
import { dirname, isAbsolute, join, resolve } from "node:path";

export const APPINDICATOR_SONAMES = [
  "libayatana-appindicator3.so.1",
  "libayatana-indicator3.so.7",
  "libayatana-ido3-0.4.so.0",
  "libdbusmenu-glib.so.4",
  "libdbusmenu-gtk3.so.4",
];

function command(program, args) {
  const result = spawnSync(program, args, {
    encoding: "utf8",
    env: { ...process.env, LC_ALL: "C" },
  });
  if (result.error) throw result.error;
  if (result.status !== 0) {
    throw new Error(`${program} failed: ${(result.stderr || result.stdout).trim()}`);
  }
  return result.stdout.trim();
}

function requireElf(path) {
  if (!statSync(path).isFile()) throw new Error(`AppIndicator runtime library is not a regular file: ${path}`);
  const file = openSync(path, "r");
  try {
    const magic = Buffer.alloc(4);
    if (readSync(file, magic, 0, 4, 0) !== 4 || !magic.equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46]))) {
      throw new Error(`AppIndicator runtime library is not ELF: ${path}`);
    }
  } finally {
    closeSync(file);
  }
}

export function requireResolvedDependencies(output) {
  const missing = output.split(/\r?\n/).filter((line) => /=>\s+not found(?:\s|$)/.test(line));
  if (missing.length) throw new Error(`AppIndicator runtime dependencies are missing:\n${missing.join("\n")}`);
}

export function appImageLibraryConfig(libraryDirectory, sonameFor = (path) => command("patchelf", ["--print-soname", path])) {
  if (!isAbsolute(libraryDirectory)) throw new Error(`pkg-config returned a non-absolute library directory: ${libraryDirectory}`);
  const files = {};
  for (const soname of APPINDICATOR_SONAMES) {
    const path = join(libraryDirectory, soname);
    let source;
    try {
      source = realpathSync(path);
    } catch (error) {
      throw new Error(`Missing AppIndicator runtime library ${path}; install libayatana-appindicator3-dev and its dependencies`, { cause: error });
    }
    requireElf(source);
    const actualSoname = sonameFor(source);
    if (actualSoname !== soname) {
      throw new Error(`Unexpected SONAME for ${source}: expected ${soname}, got ${actualSoname}`);
    }
    // Copy the real ELF under its SONAME. No absolute or dangling symlink from
    // the build host can escape into the portable package.
    files[`/usr/lib/${soname}`] = source;
  }
  return { bundle: { linux: { appimage: { files } } } };
}

export function prepareLinuxAppImage(outputPath) {
  if (process.platform !== "linux") throw new Error("AppImage runtime preparation must run on the Linux build host");
  const libraryDirectory = command("pkg-config", ["--variable=libdir", "ayatana-appindicator3-0.1"]);
  const config = appImageLibraryConfig(libraryDirectory);
  const mainLibrary = config.bundle.linux.appimage.files[`/usr/lib/${APPINDICATOR_SONAMES[0]}`];
  // ldd includes the transitive closure; fail before building if any installed
  // dependency is unresolved. Only the five missing dlopen libraries are added
  // explicitly: linuxdeploy retains its standard system-library exclusions.
  requireResolvedDependencies(command("ldd", [mainLibrary]));
  const output = resolve(outputPath);
  mkdirSync(dirname(output), { recursive: true });
  writeFileSync(output, `${JSON.stringify(config, null, 2)}\n`);
  return output;
}

if (import.meta.main) {
  try {
    const args = process.argv.slice(2);
    if (args.length !== 2 || args[0] !== "--out" || !args[1]) {
      throw new Error("Usage: bun scripts/prepare-linux-appimage.mjs --out <temporary-tauri-config.json>");
    }
    console.log(`Linux AppImage runtime overlay: ${prepareLinuxAppImage(args[1])}`);
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
