// Decides whether a pull request is only a release: a version bump written by
// `version:bump` / `version:set`, plus the changelog. CI runs light checks for
// such a pull request and the full suite for anything else; the push to main
// after merging always runs the full suite. Any doubt means "not a release".
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { CHANGELOG_FILE } from "./changelog.mjs";
import { checkVersions, compareVersions, planVersionWrite, versionFiles, versionInputs } from "./version.mjs";

const notRelease = (reason) => ({ release: false, reason });

/** Writes the version inputs of `ref` into a new temporary directory. */
function exportVersionInputs(git, ref) {
  const dir = mkdtempSync(join(tmpdir(), "mailx-release-pr-"));
  const write = (file) => {
    let text;
    try {
      text = git(["show", `${ref}:${file}`]);
    } catch {
      return false;
    }
    mkdirSync(dirname(join(dir, file)), { recursive: true });
    writeFileSync(join(dir, file), text);
    return true;
  };
  // The manifests name the other inputs (workspaces, Cargo members).
  for (const file of ["package.json", "Cargo.toml"]) write(file);
  for (const file of versionInputs(dir)) {
    if (!existsSync(join(dir, file)) && !write(file)) {
      rmSync(dir, { recursive: true, force: true });
      throw new Error(`${ref} 中缺少 ${file}`);
    }
  }
  return dir;
}

/**
 * `git` runs a git command in the repository and returns its stdout.
 * Returns `{ release, reason }`; throws when git or a file cannot be read.
 */
export function classifyReleaseChange(git, base, head) {
  const mergeBase = git(["merge-base", base, head]).trim();
  const changed = git(["diff", "--name-only", "-z", mergeBase, head]).split("\0").filter(Boolean);
  if (changed.length === 0) return notRelease("没有改动");

  const dirs = [];
  try {
    const baseDir = exportVersionInputs(git, mergeBase);
    dirs.push(baseDir);
    const headDir = exportVersionInputs(git, head);
    dirs.push(headDir);

    const files = versionFiles(baseDir);
    const allowed = new Set([...files, CHANGELOG_FILE]);
    const other = changed.filter((file) => !allowed.has(file));
    if (other.length > 0) {
      const shown = other.slice(0, 5).join(", ");
      return notRelease(`改动了版本文件以外的内容：${shown}${other.length > 5 ? ` 等 ${other.length} 个文件` : ""}`);
    }
    if (!changed.includes("package.json")) return notRelease("版本号没有变化");

    const before = checkVersions(baseDir);
    const after = checkVersions(headDir);
    if (!before.ok || !after.ok) return notRelease("版本号不一致");
    if (compareVersions(after.version, before.version) <= 0) {
      return notRelease(`版本没有升高（${before.version} → ${after.version}）`);
    }

    // Each version file must be exactly what a bump from the base writes.
    for (const [file, text] of planVersionWrite(baseDir, after.version)) {
      if (readFileSync(join(headDir, file), "utf8") !== text) {
        return notRelease(`${file} 除版本号外还有其他改动`);
      }
    }
    return { release: true, reason: `版本 ${before.version} → ${after.version}`, version: after.version };
  } finally {
    for (const dir of dirs) rmSync(dir, { recursive: true, force: true });
  }
}

/** A `git` runner for `classifyReleaseChange`, bound to a repository directory. */
export const gitIn = (cwd) => (args) =>
  execFileSync("git", args, { cwd, encoding: "utf8", maxBuffer: 64 * 1024 * 1024, stdio: ["ignore", "pipe", "pipe"] });
