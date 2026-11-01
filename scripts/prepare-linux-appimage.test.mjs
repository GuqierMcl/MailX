import { afterEach, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, realpathSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, basename } from "node:path";
import { APPINDICATOR_SONAMES, appImageLibraryConfig, requireResolvedDependencies } from "./prepare-linux-appimage.mjs";

const directories = [];
afterEach(() => {
  for (const directory of directories.splice(0)) rmSync(directory, { recursive: true, force: true });
});

function libraryDirectory() {
  const directory = mkdtempSync(join(tmpdir(), "mailx-appimage-libraries-"));
  directories.push(directory);
  return directory;
}

test("rejects a missing transitive runtime dependency before an AppImage can be built", () => {
  expect(() => requireResolvedDependencies(`
    linux-vdso.so.1 (0x00007ffdf7ac5000)
    libayatana-indicator3.so.7 => /usr/lib/x86_64-linux-gnu/libayatana-indicator3.so.7 (0x00007f93)
    libdbusmenu-gtk3.so.4 => not found
    libc.so.6 => /lib/x86_64-linux-gnu/libc.so.6 (0x00007f94)
  `)).toThrow("libdbusmenu-gtk3.so.4 => not found");
});

test("rejects a missing SONAME file even when unrelated libraries exist", () => {
  const directory = libraryDirectory();
  writeFileSync(join(directory, "unrelated.so"), "unrelated");
  expect(() => appImageLibraryConfig(directory)).toThrow("Missing AppIndicator runtime library");
});

test("rejects a directory or a non-ELF file masquerading as a runtime library", () => {
  const directory = libraryDirectory();
  const path = join(directory, APPINDICATOR_SONAMES[0]);
  mkdirSync(path);
  expect(() => appImageLibraryConfig(directory)).toThrow("not a regular file");
  rmSync(path, { recursive: true });
  writeFileSync(path, "not ELF");
  expect(() => appImageLibraryConfig(directory)).toThrow("not ELF");
});

test("rejects a library whose embedded SONAME does not match the loader's requested name", () => {
  const directory = libraryDirectory();
  writeFileSync(join(directory, APPINDICATOR_SONAMES[0]), Buffer.from([0x7f, 0x45, 0x4c, 0x46]));
  expect(() => appImageLibraryConfig(directory, () => "libwrong.so.99")).toThrow("Unexpected SONAME");
});

test.skipIf(process.platform === "win32")("resolves a versioned symlink chain so the overlay cannot contain a build-host symlink", () => {
  const directory = libraryDirectory();
  for (const soname of APPINDICATOR_SONAMES) {
    const versionedName = `${soname}.2.3`;
    writeFileSync(join(directory, versionedName), Buffer.from([0x7f, 0x45, 0x4c, 0x46]));
    symlinkSync(versionedName, join(directory, soname));
  }
  const config = appImageLibraryConfig(directory, (path) => basename(path).slice(0, -".2.3".length));
  for (const [destination, source] of Object.entries(config.bundle.linux.appimage.files)) {
    expect(source).toBe(realpathSync(join(directory, `${basename(destination)}.2.3`)));
  }
});
