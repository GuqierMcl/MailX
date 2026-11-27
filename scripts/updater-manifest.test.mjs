import { afterEach, describe, expect, test } from "bun:test";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { MissingPlatforms, PLATFORMS, buildManifest, notesForUpdater } from "./updater-manifest.mjs";

const assets = [
  "MailX_0.2.0_macos-arm64.app.tar.gz",
  "MailX_0.2.0_macos-x64.app.tar.gz",
  "MailX_0.2.0_windows-x64-setup.exe",
  "MailX_0.2.0_linux-x64.AppImage",
  "MailX_0.2.0_linux-arm64.AppImage",
];
const all = assets.flatMap((name) => [name, `${name}.sig`]);

const manifest = (files = all, overrides = {}) =>
  buildManifest({
    version: "0.2.0",
    notes: "- 修复",
    pubDate: "2026-11-01T08:00:00Z",
    baseUrl: "https://github.com/GuqierMcl/MailX/releases/download/v0.2.0",
    files,
    readSignature: (name) => `signature of ${name}\n`,
    ...overrides,
  });

describe("latest.json", () => {
  test("names every desktop platform with its signature and download URL", () => {
    const built = manifest();
    expect(Object.keys(built.platforms).sort()).toEqual(PLATFORMS.map((platform) => platform.key).sort());
    expect(built.version).toBe("0.2.0");
    expect(built.pub_date).toBe("2026-11-01T08:00:00Z");
    expect(built.platforms["darwin-aarch64"]).toEqual({
      signature: "signature of MailX_0.2.0_macos-arm64.app.tar.gz.sig",
      url: "https://github.com/GuqierMcl/MailX/releases/download/v0.2.0/MailX_0.2.0_macos-arm64.app.tar.gz",
    });
    expect(built.platforms["windows-x86_64"].url).toEndWith("MailX_0.2.0_windows-x64-setup.exe");
  });

  test("pairs each Linux architecture with its signed AppImage regardless of file order", () => {
    const baseUrl = "https://example.com/download/v0.2.0";
    for (const files of [all, [...all].reverse()]) {
      const built = manifest(files, { baseUrl });
      expect(built.platforms["linux-x86_64"]).toEqual({
        signature: "signature of MailX_0.2.0_linux-x64.AppImage.sig",
        url: `${baseUrl}/MailX_0.2.0_linux-x64.AppImage`,
      });
      expect(built.platforms["linux-aarch64"]).toEqual({
        signature: "signature of MailX_0.2.0_linux-arm64.AppImage.sig",
        url: `${baseUrl}/MailX_0.2.0_linux-arm64.AppImage`,
      });
    }
  });

  test("fails, naming what is missing, when a platform has no bundle or signature", () => {
    const cases = [
      [all.filter((name) => !name.includes("macos-arm64")), ["darwin-aarch64"]],
      [all.filter((name) => name !== "MailX_0.2.0_windows-x64-setup.exe.sig"), ["windows-x86_64"]],
      [all.filter((name) => !name.includes("macos-x64")), ["darwin-x86_64"]],
      [all.filter((name) => name !== "MailX_0.2.0_linux-x64.AppImage"), ["linux-x86_64"]],
      [all.filter((name) => name !== "MailX_0.2.0_linux-arm64.AppImage.sig"), ["linux-aarch64"]],
      [all.map((name) => name.includes("linux-arm64") ? name.replace("0.2.0", "0.1.0") : name), ["linux-aarch64"]],
      [[], PLATFORMS.map((platform) => platform.key)],
    ];
    for (const [files, missing] of cases) {
      let error = null;
      try {
        manifest(files);
      } catch (caught) {
        error = caught;
      }
      expect(error).toBeInstanceOf(MissingPlatforms);
      expect(error.keys.sort()).toEqual([...missing].sort());
    }
  });

  test("ignores the other installers that ships in the same directory", () => {
    const built = manifest([
      ...all,
      "MailX_0.2.0_macos-x64.dmg",
      "MailX_0.2.0_linux-x64.deb",
      "MailX_0.2.0_linux-x64.rpm",
      "MailX_0.2.0_linux-arm64.deb",
      "MailX_0.2.0_linux-arm64.rpm",
      "SHA256SUMS.txt",
    ]);
    expect(built.platforms["linux-x86_64"].url).toEndWith(".AppImage");
    expect(built.platforms["linux-aarch64"].url).toEndWith(".AppImage");
  });

  test("notes come from the changelog without the prerelease banner or install footer", () => {
    const changelog = [
      "# 更新日志",
      "",
      "## [Unreleased]",
      "",
      "## [0.2.0] - 2026-11-01",
      "",
      "> **预发布版本**：仍在开发中。",
      "",
      "### Feature",
      "",
      "- 应用内更新。",
      "",
      "---",
      "",
      "## 安装说明",
      "",
      "- 下载并安装。",
      "",
      "[Unreleased]: https://github.com/GuqierMcl/MailX/compare/v0.2.0...HEAD",
    ].join("\n");
    expect(notesForUpdater(changelog, "0.2.0")).toBe("### Feature\n\n- 应用内更新。");
    // A prerelease publishes whatever sits under Unreleased.
    const prerelease = changelog.replace("## [Unreleased]", "## [Unreleased]\n\n### Fixed\n\n- 一个修复。");
    expect(notesForUpdater(prerelease, "0.2.0-alpha.3")).toBe("### Fixed\n\n- 一个修复。");
    expect(notesForUpdater(changelog, "0.3.0")).toBeNull();
  });
});

describe("the command line", () => {
  let directory;
  afterEach(() => directory && rmSync(directory, { recursive: true, force: true }));

  test("writes latest.json from the files in the directory", () => {
    directory = mkdtempSync(join(tmpdir(), "mailx-updater-"));
    for (const name of all) writeFileSync(join(directory, name), `sig: ${name}`);
    const notes = join(directory, "notes.md");
    writeFileSync(notes, "## [0.2.0] - 2026-11-01\n\n- 更新。\n");
    const result = Bun.spawnSync({
      cmd: [
        "bun", "scripts/updater-manifest.mjs",
        "--version", "0.2.0",
        "--dir", directory,
        "--base-url", "https://example.com/download/v0.2.0",
        "--pub-date", "2026-11-01T08:00:00Z",
        "--notes", notes,
      ],
      cwd: import.meta.dirname + "/..",
      stderr: "pipe",
      stdout: "pipe",
    });
    expect(result.exitCode).toBe(0);
    const written = JSON.parse(readFileSync(join(directory, "latest.json"), "utf8"));
    expect(written.notes).toBe("- 更新。");
    expect(Object.keys(written.platforms)).toHaveLength(5);
  });

  test("exits non-zero without writing a partial manifest", () => {
    directory = mkdtempSync(join(tmpdir(), "mailx-updater-"));
    writeFileSync(join(directory, "MailX_0.2.0_windows-x64-setup.exe"), "binary");
    writeFileSync(join(directory, "MailX_0.2.0_windows-x64-setup.exe.sig"), "signature");
    const result = Bun.spawnSync({
      cmd: ["bun", "scripts/updater-manifest.mjs", "--version", "0.2.0", "--dir", directory, "--base-url", "https://example.com/v0.2.0"],
      cwd: import.meta.dirname + "/..",
      stderr: "pipe",
      stdout: "pipe",
    });
    expect(result.exitCode).toBe(1);
    expect(result.stderr.toString()).toContain("darwin-aarch64");
    expect(() => readFileSync(join(directory, "latest.json"))).toThrow();
  });
});
