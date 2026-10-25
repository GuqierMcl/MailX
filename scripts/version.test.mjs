import { afterEach, describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { hasEntries, releaseNotes, releaseProblems, sectionBody, stampRelease } from "./lib/changelog.mjs";
import {
  bumpVersion,
  checkVersions,
  compareVersions,
  parseVersion,
  planVersionWrite,
  repoRoot,
} from "./lib/version.mjs";

describe("parseVersion / compareVersions", () => {
  test("accepts stable and alpha/beta/rc versions only", () => {
    for (const ok of ["0.1.0", "1.20.3", "1.0.0-alpha.1", "1.0.0-beta.12", "1.0.0-rc.65535"]) {
      expect(() => parseVersion(ok)).not.toThrow();
    }
    for (const bad of ["1.0", "v1.0.0", "01.0.0", "1.0.0-alpha", "1.0.0-alpha.0", "1.0.0-dev.1", "1.0.0-rc.65536", "1.0.0+build"]) {
      expect(() => parseVersion(bad)).toThrow();
    }
  });

  test("orders by SemVer precedence", () => {
    const ascending = ["0.1.0-alpha.1", "0.1.0-alpha.2", "0.1.0-beta.1", "0.1.0-rc.1", "0.1.0", "0.1.1-alpha.1", "0.2.0"];
    for (let i = 1; i < ascending.length; i++) {
      expect(compareVersions(ascending[i - 1], ascending[i])).toBeLessThan(0);
      expect(compareVersions(ascending[i], ascending[i - 1])).toBeGreaterThan(0);
    }
    expect(compareVersions("1.2.3", "1.2.3")).toBe(0);
  });
});

describe("bumpVersion", () => {
  test.each([
    ["0.1.0", "patch", undefined, "0.1.1"],
    ["0.1.5", "minor", undefined, "0.2.0"],
    ["0.9.5", "major", undefined, "1.0.0"],
    ["0.1.0", "prepatch", "alpha", "0.1.1-alpha.1"],
    ["0.1.3", "preminor", "beta", "0.2.0-beta.1"],
    ["0.9.0", "premajor", "rc", "1.0.0-rc.1"],
    ["0.2.0-alpha.1", "pre", undefined, "0.2.0-alpha.2"],
    ["0.2.0-alpha.2", "pre", "alpha", "0.2.0-alpha.3"],
    ["0.2.0-alpha.2", "pre", "beta", "0.2.0-beta.1"],
    ["0.2.0-beta.4", "pre", "rc", "0.2.0-rc.1"],
    ["0.2.0-rc.2", "release", undefined, "0.2.0"],
  ])("%s %s %s -> %s", (current, command, id, expected) => {
    expect(bumpVersion(current, command, id)).toBe(expected);
  });

  test("refuses ambiguous or backwards moves", () => {
    expect(() => bumpVersion("1.0.0-rc.1", "patch")).toThrow();
    expect(() => bumpVersion("1.0.0-rc.1", "prepatch", "alpha")).toThrow();
    expect(() => bumpVersion("1.0.0", "pre")).toThrow();
    expect(() => bumpVersion("1.0.0", "release")).toThrow();
    expect(() => bumpVersion("1.0.0-rc.1", "pre", "beta")).toThrow();
    expect(() => bumpVersion("1.0.0", "prepatch")).toThrow();
    expect(() => bumpVersion("1.0.0", "prepatch", "dev")).toThrow();
    expect(() => bumpVersion("1.0.0", "bogus")).toThrow();
    expect(() => bumpVersion("1.0.0-alpha.65535", "pre")).toThrow();
  });
});

const CHANGELOG = `# 更新日志

<!-- 说明 -->

## [Unreleased]

### 新增

- 新功能 A

## [0.1.0] - 2026-01-01

### 新增

- 初始版本

[Unreleased]: https://example.test/o/r/compare/v0.1.0...HEAD
[0.1.0]: https://example.test/o/r/releases/tag/v0.1.0
`;

describe("changelog", () => {
  test("reads sections up to the next section or the link definitions", () => {
    expect(sectionBody(CHANGELOG, "Unreleased")).toBe("### 新增\n\n- 新功能 A");
    expect(sectionBody(CHANGELOG, "0.1.0")).toBe("### 新增\n\n- 初始版本");
    expect(sectionBody(CHANGELOG, "9.9.9")).toBeNull();
  });

  test("entries need a list item; comments and bare headings do not count", () => {
    expect(hasEntries(null)).toBe(false);
    expect(hasEntries("### 新增\n\n<!-- - 注释 -->")).toBe(false);
    expect(hasEntries("### 新增\n\n- 内容")).toBe(true);
  });

  test("stamps Unreleased into a dated section and moves the links", () => {
    const stamped = stampRelease(CHANGELOG, "0.2.0", "2026-02-02", "https://example.test/o/r");
    expect(stamped).toContain("## [Unreleased]\n\n## [0.2.0] - 2026-02-02\n\n### 新增\n\n- 新功能 A");
    expect(stamped).toContain("[Unreleased]: https://example.test/o/r/compare/v0.2.0...HEAD");
    expect(stamped).toContain("[0.2.0]: https://example.test/o/r/compare/v0.1.0...v0.2.0");
    expect(stamped).toContain("[0.1.0]: https://example.test/o/r/releases/tag/v0.1.0");
    expect(hasEntries(sectionBody(stamped, "Unreleased"))).toBe(false);
  });

  test("the first release links to its tag and adds missing link definitions", () => {
    const first = "## [Unreleased]\n\n- x\n";
    const stamped = stampRelease(first, "0.1.0", "2026-01-01", "https://example.test/o/r");
    expect(stamped).toContain("[Unreleased]: https://example.test/o/r/compare/v0.1.0...HEAD");
    expect(stamped).toContain("[0.1.0]: https://example.test/o/r/releases/tag/v0.1.0");
  });

  test("refuses an empty Unreleased, a duplicate version and prereleases", () => {
    const empty = "## [Unreleased]\n\n## [0.1.0] - 2026-01-01\n\n- x\n";
    expect(() => stampRelease(empty, "0.2.0", "2026-02-02", "u")).toThrow();
    expect(() => stampRelease(CHANGELOG, "0.1.0", "2026-02-02", "u")).toThrow();
    expect(() => stampRelease(CHANGELOG, "0.2.0-alpha.1", "2026-02-02", "u")).toThrow();
  });

  test("release readiness: stable needs its own section, prerelease needs Unreleased", () => {
    expect(releaseProblems(CHANGELOG, "0.1.0")).toEqual([]);
    expect(releaseProblems(CHANGELOG, "0.2.0")).toHaveLength(1);
    expect(releaseProblems(CHANGELOG, "0.2.0-beta.1")).toEqual([]);
    expect(releaseProblems("## [Unreleased]\n", "0.2.0-beta.1")).toHaveLength(1);
  });
});

describe("a project on disk", () => {
  const dirs = [];
  afterEach(() => {
    for (const dir of dirs.splice(0)) rmSync(dir, { recursive: true, force: true });
  });

  function project(version = "0.1.0") {
    const dir = mkdtempSync(join(tmpdir(), "mailx-version-"));
    dirs.push(dir);
    const write = (file, text) => {
      mkdirSync(dirname(join(dir, file)), { recursive: true });
      writeFileSync(join(dir, file), text);
    };
    write("package.json", `{\n  "name": "root",\n  "version": "${version}",\n  "repository": "https://example.test/o/r.git",\n  "workspaces": ["frontend"]\n}\n`);
    write("frontend/package.json", `{\n  "name": "fe",\n  "version": "${version}",\n  "dependencies": { "x": "1.0.0" }\n}\n`);
    write("Cargo.toml", `[workspace]\nmembers = ["core", "desktop"]\n\n[workspace.package]\nedition = "2024"\nversion = "${version}"\nauthors = ["a"]\n\n[workspace.dependencies]\nserde = "1"\n`);
    write("core/Cargo.toml", `[package]\nname = "mailx-core"\nversion.workspace = true\n`);
    write("desktop/Cargo.toml", `[package]\nname = "mailx-desktop"\nversion.workspace = true\n`);
    write(
      "Cargo.lock",
      `version = 4\n\n[[package]]\nname = "mailx-core"\nversion = "${version}"\ndependencies = [\n "serde",\n]\n\n[[package]]\nname = "mailx-desktop"\nversion = "${version}"\n\n[[package]]\nname = "serde"\nversion = "1.0.0"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n`,
    );
    return dir;
  }

  test("reads, checks and rewrites every recorded version, nothing else", () => {
    const dir = project();
    expect(checkVersions(dir).ok).toBe(true);
    const files = planVersionWrite(dir, "0.2.0-beta.1");
    expect([...files.keys()].sort()).toEqual(["Cargo.lock", "Cargo.toml", "frontend/package.json", "package.json"]);
    for (const [file, text] of files) writeFileSync(join(dir, file), text);

    const after = checkVersions(dir);
    expect(after.ok && after.version).toBe("0.2.0-beta.1");
    expect(after.entries).toHaveLength(5);
    // Other versions in the same files stay untouched.
    expect(files.get("Cargo.lock")).toContain('name = "serde"\nversion = "1.0.0"\nsource');
    expect(files.get("Cargo.toml")).toContain('serde = "1"');
    expect(files.get("Cargo.toml")).toContain('edition = "2024"');
    expect(files.get("frontend/package.json")).toContain('"x": "1.0.0"');
  });

  test("reports a mismatch", () => {
    const dir = project();
    writeFileSync(join(dir, "frontend/package.json"), '{ "version": "0.3.0" }\n');
    expect(checkVersions(dir).ok).toBe(false);
  });

  test("a member that does not inherit the workspace version is rejected", () => {
    const dir = project();
    writeFileSync(join(dir, "core/Cargo.toml"), `[package]\nname = "mailx-core"\nversion = "0.1.0"\n`);
    expect(() => checkVersions(dir)).toThrow();
  });

  test("release notes: prerelease banner, section body, compare link, install footer", () => {
    const dir = project();
    writeFileSync(join(dir, "CHANGELOG.md"), CHANGELOG);
    mkdirSync(join(dir, ".github"));
    writeFileSync(join(dir, ".github/release-install-notes.md"), "## 安装说明\n\n脚注\n");

    const pre = releaseNotes(dir, "0.2.0-alpha.1");
    expect(pre).toStartWith("> **预发布版本**");
    expect(pre).toContain("- 新功能 A");
    expect(pre).toEndWith("---\n\n## 安装说明\n\n脚注\n");

    const stable = stampRelease(CHANGELOG, "0.2.0", "2026-02-02", "https://example.test/o/r");
    writeFileSync(join(dir, "CHANGELOG.md"), stable);
    const notes = releaseNotes(dir, "0.2.0");
    expect(notes).not.toContain("预发布版本");
    expect(notes).toContain("**完整变更**：https://example.test/o/r/compare/v0.1.0...v0.2.0");
    expect(() => releaseNotes(dir, "0.3.0")).toThrow();
  });
});

describe("this repository", () => {
  test("every recorded version agrees and prerelease notes can be built", () => {
    expect(checkVersions(repoRoot).ok).toBe(true);
    expect(() => releaseNotes(repoRoot, "0.0.0-alpha.1")).not.toThrow();
  });
});
