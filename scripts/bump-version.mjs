import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { CHANGELOG_FILE, stampRelease } from "./lib/changelog.mjs";
import {
  PRERELEASE_IDS,
  bumpVersion,
  checkVersions,
  compareVersions,
  isPrerelease,
  parseVersion,
  planVersionWrite,
  repoRoot,
  repositoryUrl,
} from "./lib/version.mjs";

const USAGE = `用法：
  bun run version:bump <命令> [${PRERELEASE_IDS.join("|")}] [选项]
  bun run version:set <版本号> [选项]

命令（只改文件，不提交、不打标签）：
  major | minor | patch                正式版进位：0.1.0 -> 1.0.0 / 0.2.0 / 0.1.1
  premajor | preminor | prepatch <id>  进位并进入预发布：0.1.0 -> 0.1.1-<id>.1
  pre [id]                             预发布序号加一，或切到更靠后的标识（alpha -> beta -> rc）
  release                              去掉预发布后缀：1.0.0-rc.2 -> 1.0.0
  set <版本号>                          直接设置；不高于当前版本时需要 --force

选项：
  --dry-run       只显示将要做的修改
  --no-changelog  不改 CHANGELOG.md（正式版默认把 [Unreleased] 归并为带日期的小节）
  --force         set 时允许不高于当前版本`;

function fail(message) {
  console.error(`${message}\n\n${USAGE}`);
  process.exit(1);
}

const positional = [];
const options = new Set();
for (const arg of process.argv.slice(2)) {
  if (arg.startsWith("--")) options.add(arg);
  else positional.push(arg);
}
for (const option of options) {
  if (!["--dry-run", "--no-changelog", "--force"].includes(option)) fail(`未知选项 ${option}`);
}

const [command, argument, ...extra] = positional;
if (!command) fail("缺少命令");
if (extra.length) fail(`多余的参数：${extra.join(" ")}`);

const current = checkVersions(repoRoot);
if (!current.ok) {
  fail(
    `仓库里的版本号不一致，先修复：\n${current.entries.map((e) => `  ${e.label}: ${e.version ?? "(missing)"}`).join("\n")}`,
  );
}

let next;
try {
  if (command === "set") {
    if (!argument) fail("set 需要一个版本号");
    parseVersion(argument);
    if (compareVersions(argument, current.version) <= 0 && !options.has("--force")) {
      fail(
        `${argument} 不高于当前版本 ${current.version}。` +
          `第一个预发布版（例如 0.1.0 之前的 0.1.0-alpha.1）或确需回退时加 --force`,
      );
    }
    next = argument;
  } else {
    next = bumpVersion(current.version, command, argument);
  }
} catch (error) {
  fail(error.message);
}

const tagged = spawnSync("git", ["tag", "--list", `v${next}`], { cwd: repoRoot, encoding: "utf8" });
if (tagged.status === 0 && tagged.stdout.trim()) {
  fail(`标签 v${next} 已存在；版本号不能复用`);
}

const files = planVersionWrite(repoRoot, next);
let changelog = "未改动";
if (isPrerelease(next)) {
  changelog = "未改动（预发布版沿用 [Unreleased]）";
} else if (!options.has("--no-changelog")) {
  const now = new Date();
  const date = [now.getFullYear(), now.getMonth() + 1, now.getDate()]
    .map((part, index) => String(part).padStart(index === 0 ? 4 : 2, "0"))
    .join("-");
  try {
    const text = readFileSync(resolve(repoRoot, CHANGELOG_FILE), "utf8");
    files.set(CHANGELOG_FILE, stampRelease(text, next, date, repositoryUrl(repoRoot)));
    changelog = `[Unreleased] 归并为 [${next}] - ${date}`;
  } catch (error) {
    fail(error.message);
  }
}

console.log(`${current.version} -> ${next}${options.has("--dry-run") ? "（dry run，未写入）" : ""}`);
console.log(`文件：${[...files.keys()].join(", ")}`);
console.log(`更新日志：${changelog}`);
if (options.has("--dry-run")) process.exit(0);

for (const [file, text] of files) writeFileSync(resolve(repoRoot, file), text);
const after = checkVersions(repoRoot);
if (!after.ok || after.version !== next) {
  console.error("写入后版本号校验失败，请用 git diff 检查并还原");
  process.exit(1);
}
console.log(`
下一步：
  git diff                                   检查改动
  bun run check && bun run rust:test         本地验证（也可交给 CI）
  git commit -am "chore(release): v${next}"  经 PR 合并到 main 后再打标签 v${next}`);
