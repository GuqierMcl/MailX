import { afterEach, describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { PLATFORMS, UnknownPlatform, renameAssets, renamed } from "./release-assets.mjs";
import { MissingPlatforms, buildManifest } from "./updater-manifest.mjs";

describe("renamed", () => {
  test("names macOS, Windows and Linux installers after the platform", () => {
    expect(renamed("MailX_0.1.0-alpha.4_aarch64.dmg", "0.1.0-alpha.4", "macos-arm64")).toBe("MailX_0.1.0-alpha.4_macos-arm64.dmg");
    expect(renamed("MailX.app.tar.gz", "0.1.0-alpha.4", "macos-x64")).toBe("MailX_0.1.0-alpha.4_macos-x64.app.tar.gz");
    expect(renamed("MailX_0.1.0-alpha.4_arm64-setup.exe", "0.1.0-alpha.4", "windows-arm64")).toBe(
      "MailX_0.1.0-alpha.4_windows-arm64-setup.exe",
    );
    expect(renamed("MailX_0.1.0-alpha.4_x64-setup.exe", "0.1.0-alpha.4", "windows-x64")).toBe(
      "MailX_0.1.0-alpha.4_windows-x64-setup.exe",
    );
    expect(renamed("MailX-0.1.0-alpha.4-1.x86_64.rpm", "0.1.0-alpha.4", "linux-x64")).toBe(
      "MailX_0.1.0-alpha.4_linux-x64.rpm",
    );
    expect(renamed("MailX_0.1.0-alpha.4_amd64.AppImage", "0.1.0-alpha.4", "linux-x64")).toBe(
      "MailX_0.1.0-alpha.4_linux-x64.AppImage",
    );
    expect(renamed("MailX_0.1.0-alpha.4_arm64.deb", "0.1.0-alpha.4", "linux-arm64")).toBe(
      "MailX_0.1.0-alpha.4_linux-arm64.deb",
    );
    expect(renamed("MailX-0.1.0-alpha.4-1.aarch64.rpm", "0.1.0-alpha.4", "linux-arm64")).toBe(
      "MailX_0.1.0-alpha.4_linux-arm64.rpm",
    );
  });

  test("a signature keeps its bundle's name and gains .sig", () => {
    expect(renamed("MailX_0.1.0-alpha.4_aarch64.app.tar.gz.sig", "0.1.0-alpha.4", "macos-arm64")).toBe(
      "MailX_0.1.0-alpha.4_macos-arm64.app.tar.gz.sig",
    );
    expect(renamed("MailX_0.1.0-alpha.4_x64-setup.exe.sig", "0.1.0-alpha.4", "windows-x64")).toBe(
      "MailX_0.1.0-alpha.4_windows-x64-setup.exe.sig",
    );
  });

  test("leaves anything else alone", () => {
    for (const name of ["SHA256SUMS.txt", "latest.json", "MailX.app", "MailX_0.1.0-alpha.4_aarch64.dmg.blockmap"]) {
      expect(renamed(name, "0.1.0-alpha.4", "macos-arm64")).toBeNull();
    }
  });
});

describe("renameAssets", () => {
  let directory;
  afterEach(() => directory && rmSync(directory, { recursive: true, force: true }));

  function bundle(files) {
    directory = mkdtempSync(join(tmpdir(), "mailx-assets-"));
    for (const name of files) writeFileSync(join(directory, name), name);
    return directory;
  }

  test("renames only the collected files and reports each change", () => {
    const dir = bundle([
      "MailX_0.1.0-alpha.4_aarch64.dmg",
      "MailX.app.tar.gz",
      "MailX.app.tar.gz.sig",
      "SHA256SUMS.txt",
    ]);
    const changes = renameAssets(dir, "0.1.0-alpha.4", "macos-arm64");
    expect(changes).toHaveLength(3);
    expect(readdirSync(dir).sort()).toEqual(
      [
        "MailX_0.1.0-alpha.4_macos-arm64.app.tar.gz",
        "MailX_0.1.0-alpha.4_macos-arm64.app.tar.gz.sig",
        "MailX_0.1.0-alpha.4_macos-arm64.dmg",
        "SHA256SUMS.txt",
      ].sort(),
    );
  });

  test("the renamed assets are what the updater manifest expects", () => {
    const files = [];
    const bundles = {
      "macos-arm64": "MailX.app.tar.gz",
      "macos-x64": "MailX.app.tar.gz",
      "windows-x64": "MailX_0.1.0-alpha.4_x64-setup.exe",
      "windows-arm64": "MailX_0.1.0-alpha.4_arm64-setup.exe",
      "linux-x64": "MailX_0.1.0-alpha.4_amd64.AppImage",
      "linux-arm64": "MailX_0.1.0-alpha.4_aarch64.AppImage",
    };
    for (const platform of PLATFORMS) {
      const name = bundles[platform];
      const dir = bundle([name, `${name}.sig`]);
      renameAssets(dir, "0.1.0-alpha.4", platform);
      files.push(...readdirSync(dir));
      rmSync(dir, { recursive: true, force: true });
      directory = undefined;
    }
    const manifest = buildManifest({
      version: "0.1.0-alpha.4",
      notes: "- 更新",
      pubDate: "2026-11-01T08:00:00Z",
      baseUrl: "https://example.com/download/v0.1.0-alpha.4",
      files,
      readSignature: (name) => `sig: ${name}`,
    });
    expect(Object.keys(manifest.platforms).sort()).toEqual(["darwin-aarch64", "darwin-x86_64", "linux-aarch64", "linux-x86_64", "windows-aarch64", "windows-x86_64"]);
    expect(manifest.platforms["darwin-x86_64"].url).toEndWith("MailX_0.1.0-alpha.4_macos-x64.app.tar.gz");
    expect(manifest.platforms["darwin-aarch64"].url).toEndWith("MailX_0.1.0-alpha.4_macos-arm64.app.tar.gz");
    expect(manifest.platforms["linux-x86_64"].url).toEndWith("MailX_0.1.0-alpha.4_linux-x64.AppImage");
    expect(manifest.platforms["linux-aarch64"].url).toEndWith("MailX_0.1.0-alpha.4_linux-arm64.AppImage");
    expect(manifest.platforms["windows-x86_64"].url).toEndWith("MailX_0.1.0-alpha.4_windows-x64-setup.exe");
    expect(manifest.platforms["windows-aarch64"].url).toEndWith("MailX_0.1.0-alpha.4_windows-arm64-setup.exe");
  });

  test("an unknown platform is refused before anything moves", () => {
    const dir = bundle(["MailX_0.1.0-alpha.4_aarch64.dmg"]);
    expect(() => renameAssets(dir, "0.1.0-alpha.4", "linux-armhf")).toThrow(UnknownPlatform);
    expect(readdirSync(dir)).toEqual(["MailX_0.1.0-alpha.4_aarch64.dmg"]);
  });

  test("a bundle without its signature leaves the manifest incomplete", () => {
    const dir = bundle(["MailX_0.1.0-alpha.4_x64-setup.exe"]);
    renameAssets(dir, "0.1.0-alpha.4", "windows-x64");
    expect(() =>
      buildManifest({
        version: "0.1.0-alpha.4",
        notes: "",
        pubDate: "2026-11-01T08:00:00Z",
        baseUrl: "https://example.com",
        files: readdirSync(dir),
        readSignature: () => "",
      }),
    ).toThrow(MissingPlatforms);
  });
});

describe("the command line", () => {
  let directory;
  afterEach(() => directory && rmSync(directory, { recursive: true, force: true }));

  test("names the files in a directory", () => {
    directory = mkdtempSync(join(tmpdir(), "mailx-assets-"));
    mkdirSync(join(directory, "dist"));
    writeFileSync(join(directory, "dist", "MailX_0.1.0-alpha.4_arm64.deb"), "deb");
    const result = Bun.spawnSync({
      cmd: ["bun", "scripts/release-assets.mjs", "--dir", join(directory, "dist"), "--version", "0.1.0-alpha.4", "--platform", "linux-arm64"],
      cwd: `${import.meta.dirname}/..`,
      stderr: "pipe",
      stdout: "pipe",
    });
    expect(result.exitCode).toBe(0);
    expect(readdirSync(join(directory, "dist"))).toEqual(["MailX_0.1.0-alpha.4_linux-arm64.deb"]);
  });

  test("exits non-zero for an unknown platform", () => {
    directory = mkdtempSync(join(tmpdir(), "mailx-assets-"));
    const result = Bun.spawnSync({
      cmd: ["bun", "scripts/release-assets.mjs", "--dir", directory, "--version", "0.1.0-alpha.4", "--platform", "bsd-x64"],
      cwd: `${import.meta.dirname}/..`,
      stderr: "pipe",
      stdout: "pipe",
    });
    expect(result.exitCode).toBe(1);
    expect(result.stderr.toString()).toContain("未知的发布平台");
  });
});
