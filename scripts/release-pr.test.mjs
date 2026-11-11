import { afterEach, describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { classifyReleaseChange, gitIn } from "./lib/release-pr.mjs";
import { planVersionWrite } from "./lib/version.mjs";

describe("classifyReleaseChange", () => {
  const dirs = [];
  afterEach(() => {
    for (const dir of dirs.splice(0)) rmSync(dir, { recursive: true, force: true });
  });

  /** A git repository holding a small two-member project at 0.1.0, on `main`. */
  function repository() {
    const dir = mkdtempSync(join(tmpdir(), "mailx-release-pr-test-"));
    dirs.push(dir);
    const git = gitIn(dir);
    const write = (file, text) => {
      mkdirSync(dirname(join(dir, file)), { recursive: true });
      writeFileSync(join(dir, file), text);
    };
    const commit = (message) => {
      git(["add", "-A"]);
      git(["-c", "user.name=t", "-c", "user.email=t@example.test", "commit", "-q", "-m", message]);
      return git(["rev-parse", "HEAD"]).trim();
    };
    git(["init", "-q", "-b", "main"]);
    write("package.json", `{\n  "name": "root",\n  "version": "0.1.0",\n  "repository": "https://example.test/o/r.git",\n  "workspaces": ["frontend"]\n}\n`);
    write("frontend/package.json", `{\n  "name": "fe",\n  "version": "0.1.0",\n  "dependencies": { "x": "1.0.0" }\n}\n`);
    write("Cargo.toml", `[workspace]\nmembers = ["core", "desktop"]\n\n[workspace.package]\nversion = "0.1.0"\n`);
    write("core/Cargo.toml", `[package]\nname = "mailx-core"\nversion.workspace = true\n`);
    write("desktop/Cargo.toml", `[package]\nname = "mailx-desktop"\nversion.workspace = true\n`);
    write(
      "Cargo.lock",
      `version = 4\n\n[[package]]\nname = "mailx-core"\nversion = "0.1.0"\n\n[[package]]\nname = "mailx-desktop"\nversion = "0.1.0"\n\n[[package]]\nname = "serde"\nversion = "1.0.0"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n`,
    );
    write("CHANGELOG.md", "# Changelog\n\n## [Unreleased]\n\n- A change.\n");
    write("src/main.rs", "fn main() {}\n");
    const base = commit("base");
    git(["switch", "-q", "-c", "release"]);
    const bump = (version) => {
      for (const [file, text] of planVersionWrite(dir, version)) write(file, text);
    };
    return { dir, git, write, commit, base, bump };
  }

  test("a version bump with its changelog is a release", () => {
    const repo = repository();
    repo.bump("0.2.0");
    repo.write("CHANGELOG.md", "# Changelog\n\n## [Unreleased]\n\n## [0.2.0] - 2026-11-12\n\n- A change.\n");
    const head = repo.commit("chore(release): v0.2.0");
    expect(classifyReleaseChange(repo.git, repo.base, head)).toMatchObject({ release: true, version: "0.2.0" });
  });

  test("a prerelease bump without changelog changes is a release", () => {
    const repo = repository();
    repo.bump("0.2.0-rc.1");
    const head = repo.commit("chore(release): v0.2.0-rc.1");
    expect(classifyReleaseChange(repo.git, repo.base, head).release).toBe(true);
  });

  test("a bump that also touches any other file is not", () => {
    const repo = repository();
    repo.bump("0.2.0");
    repo.write("src/main.rs", "fn main() { println!(); }\n");
    const head = repo.commit("bump and code");
    expect(classifyReleaseChange(repo.git, repo.base, head)).toMatchObject({ release: false });
  });

  test("changing a member manifest is not", () => {
    const repo = repository();
    repo.bump("0.2.0");
    repo.write("core/Cargo.toml", `[package]\nname = "mailx-core"\nversion.workspace = true\n\n[dependencies]\nevil = "1"\n`);
    expect(classifyReleaseChange(repo.git, repo.base, repo.commit("x")).release).toBe(false);
  });

  test("other changes inside a version file are not", () => {
    for (const [file, from, to] of [
      ["frontend/package.json", `"x": "1.0.0"`, `"x": "2.0.0"`],
      ["Cargo.lock", `name = "serde"\nversion = "1.0.0"`, `name = "serde"\nversion = "1.0.1"`],
      ["Cargo.toml", `[workspace]\n`, `[workspace]\nresolver = "2"\n`],
    ]) {
      const repo = repository();
      repo.bump("0.2.0");
      const text = readFileSync(join(repo.dir, file), "utf8");
      expect(text.includes(from)).toBe(true);
      repo.write(file, text.replace(from, to));
      const result = classifyReleaseChange(repo.git, repo.base, repo.commit("x"));
      expect(result).toMatchObject({ release: false });
      expect(result.reason).toContain(file);
    }
  });

  test("a lower, equal or inconsistent version is not", () => {
    const lower = repository();
    lower.bump("0.0.9");
    expect(classifyReleaseChange(lower.git, lower.base, lower.commit("x")).release).toBe(false);

    const partial = repository();
    for (const [file, text] of planVersionWrite(partial.dir, "0.2.0")) {
      if (file !== "Cargo.lock") partial.write(file, text);
    }
    expect(classifyReleaseChange(partial.git, partial.base, partial.commit("x")).release).toBe(false);
  });

  test("changelog or documentation alone is not a release", () => {
    const repo = repository();
    repo.write("CHANGELOG.md", "# Changelog\n\n## [Unreleased]\n\n- Another change.\n");
    expect(classifyReleaseChange(repo.git, repo.base, repo.commit("docs")).release).toBe(false);
  });

  test("compares against the merge base, not the moving target branch", () => {
    const repo = repository();
    repo.bump("0.2.0");
    const head = repo.commit("chore(release): v0.2.0");
    repo.git(["switch", "-q", "main"]);
    repo.write("src/main.rs", "fn main() { /* merged meanwhile */ }\n");
    const main = repo.commit("feature on main");
    expect(classifyReleaseChange(repo.git, main, head).release).toBe(true);
  });
});
