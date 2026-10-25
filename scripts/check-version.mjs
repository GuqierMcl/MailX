import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { CHANGELOG_FILE, releaseProblems } from "./lib/changelog.mjs";
import { checkVersions, isPrerelease, parseVersion, repoRoot } from "./lib/version.mjs";

// bun run version:check                  every recorded version agrees
// bun run version:check --release        ... and the changelog is ready for it
// bun run version:check --tag v1.2.3     ... and the tag names that version
const args = process.argv.slice(2);
const flag = (name) => args.includes(name);
const tagIndex = args.indexOf("--tag");
const tag = tagIndex >= 0 ? args[tagIndex + 1] : undefined;

const fail = (lines) => {
  console.error(lines.join("\n"));
  process.exit(1);
};

if (tagIndex >= 0 && !tag) fail(["--tag 需要一个标签名，例如 --tag v1.2.3"]);

const { ok, version, entries } = checkVersions(repoRoot);
if (!ok) {
  fail(["Version mismatch:", ...entries.map((entry) => `  ${entry.label}: ${entry.version ?? "(missing)"}`)]);
}

const problems = [];
try {
  parseVersion(version);
} catch (error) {
  problems.push(error.message);
}
if (tag !== undefined && tag !== `v${version}`) {
  problems.push(`标签 ${tag} 与仓库版本 v${version} 不一致`);
}
if (flag("--release") && problems.length === 0) {
  const changelog = readFileSync(resolve(repoRoot, CHANGELOG_FILE), "utf8");
  problems.push(...releaseProblems(changelog, version));
}
if (problems.length) fail(problems);

const kind = isPrerelease(version) ? "prerelease" : "stable";
console.log(`MailX version ${version} (${kind}; package.json files, Cargo workspace, Cargo.lock)`);
