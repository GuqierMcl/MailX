import { afterAll, describe, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { changeScope } from "./lib/change-scope.mjs";
import { gitIn } from "./lib/release-pr.mjs";

describe("changeScope", () => {
  const cases = [
    [["website/src/pages/en/index.astro"], "site", true, true],
    [["website/package.json", "docs/guides/en/index.md", "docs/assets/screenshots/en/inbox-dark.webp"], "site", true, true],
    [["website/README.md", "docs/architecture/overview.md"], "site", true, false],
    [["README.md", "docs/architecture/overview.md"], "docs", false, false],
    [["CHANGELOG.md"], "docs", true, false],
    [["website/src/a.ts", "frontend/src/App.tsx"], "code", true, false],
    [["website/package.json", "bun.lock"], "code", true, false],
    [["frontend/public/favicon.svg"], "code", true, false],
    [["core/src/lib.rs"], "code", false, false],
    [["scripts/check-change-scope.mjs"], "code", false, false],
    [[".github/workflows/ci.yml"], "code", true, false],
    [["website-extra/a.ts"], "code", false, false],
    [["core/website/a.rs"], "code", false, false],
    [[], "code", true, false],
  ];
  for (const [files, scope, website, siteOnly] of cases) {
    test(files.join(", ") || "no files", () => expect(changeScope(files)).toEqual({ scope, website, siteOnly }));
  }
});

describe("check-change-scope", () => {
  const dir = mkdtempSync(join(tmpdir(), "mailx-change-scope-test-"));
  afterAll(() => rmSync(dir, { recursive: true, force: true }));
  const git = gitIn(dir);
  const commit = () => {
    git(["add", "-A"]);
    git(["-c", "user.name=t", "-c", "user.email=t@example.test", "commit", "-q", "-m", "c"]);
    return git(["rev-parse", "HEAD"]).trim();
  };
  const run = (range) => Bun.spawnSync([process.execPath, resolve(import.meta.dirname, "check-change-scope.mjs"), range], { cwd: dir });

  test("a file moved from the application into the website is code", () => {
    git(["init", "-q", "-b", "main"]);
    mkdirSync(join(dir, "core"));
    mkdirSync(join(dir, "website"));
    writeFileSync(join(dir, "core/a.rs"), "fn main() {}\n".repeat(20));
    const base = commit();
    renameSync(join(dir, "core/a.rs"), join(dir, "website/a.rs"));
    const head = commit();
    expect(run(`${base}...${head}`).stdout.toString()).toContain("scope=code\n");
    writeFileSync(join(dir, "website/中文.md"), "x\n");
    const next = commit();
    expect(run(`${head}..${next}`).stdout.toString()).toContain("scope=site\nwebsite=true\nsite-only=true\n");
  });

  test("a range that cannot be read fails", () => {
    expect(run("missing...HEAD").exitCode).not.toBe(0);
  });
});
