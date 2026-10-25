// CHANGELOG.md in Keep a Changelog layout: `## [Unreleased]` on top, one
// `## [x.y.z] - date` section per stable release, link definitions at the end.
//
// Prereleases never get a section of their own. Their notes are whatever sits
// under Unreleased, and the stable release folds all of it into one section.
import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { isPrerelease } from "./version.mjs";

export const CHANGELOG_FILE = "CHANGELOG.md";
const HEADING = /^## \[([^\]]+)\].*$/gm;
// Where a section ends: the next section, or the link definitions at the end.
const SECTION_END = /^(?:## \[|\[[^\]]+\]:\s)/m;

const headings = (text) => [...text.matchAll(HEADING)];

/** Trimmed text under `## [name]`, or null when there is no such section. */
export function sectionBody(text, name) {
  const heading = headings(text).find((match) => match[1] === name);
  if (!heading) return null;
  const rest = text.slice(heading.index + heading[0].length);
  const end = rest.search(SECTION_END);
  return (end < 0 ? rest : rest.slice(0, end)).trim();
}

/** A section counts as written once it holds at least one list item. */
export const hasEntries = (body) =>
  body !== null && /^\s*[-*] \S/m.test(body.replace(/<!--[\s\S]*?-->/g, ""));

/** Fold Unreleased into a dated section for `version` and refresh the links. */
export function stampRelease(text, version, date, repoUrl) {
  if (isPrerelease(version)) throw new Error("预发布版不生成更新日志小节");
  if (!hasEntries(sectionBody(text, "Unreleased"))) {
    throw new Error("CHANGELOG.md 的 [Unreleased] 下没有任何条目；先补写更新内容（或用 --no-changelog 跳过）");
  }
  if (sectionBody(text, version) !== null) {
    throw new Error(`CHANGELOG.md 已存在 [${version}] 小节`);
  }

  const stamped = text.replace(/^## \[Unreleased\].*$/m, `## [Unreleased]\n\n## [${version}] - ${date}`);
  const previous = headings(stamped)
    .map((match) => match[1])
    .filter((name) => /^\d+\.\d+\.\d+$/.test(name))
    .find((name) => name !== version);
  const links =
    `[Unreleased]: ${repoUrl}/compare/v${version}...HEAD\n` +
    `[${version}]: ${previous ? `${repoUrl}/compare/v${previous}...v${version}` : `${repoUrl}/releases/tag/v${version}`}`;
  return /^\[Unreleased\]:.*$/m.test(stamped)
    ? stamped.replace(/^\[Unreleased\]:.*$/m, links)
    : `${stamped.replace(/\n*$/, "")}\n\n${links}\n`;
}

/** Reasons this version cannot be released from the current changelog. */
export function releaseProblems(text, version) {
  const name = isPrerelease(version) ? "Unreleased" : version;
  return hasEntries(sectionBody(text, name))
    ? []
    : [`CHANGELOG.md 的 [${name}] 小节没有任何条目（以 "- " 开头的列表项）`];
}

const PRERELEASE_NOTICE =
  "> **预发布版本**：仍在开发中，功能不完整，也未经真实邮件服务商的完整兼容性验证。请勿用于唯一的邮箱工作流。";

/** The GitHub Release body for `version`. */
export function releaseNotes(dir, version) {
  const text = readFileSync(resolve(dir, CHANGELOG_FILE), "utf8");
  const prerelease = isPrerelease(version);
  const problems = releaseProblems(text, version);
  if (problems.length) throw new Error(problems.join("\n"));

  const parts = [];
  if (prerelease) parts.push(PRERELEASE_NOTICE);
  parts.push(sectionBody(text, prerelease ? "Unreleased" : version));
  const link = new RegExp(`^\\[${version.replace(/\./g, "\\.")}\\]:\\s+(\\S+)`, "m").exec(text)?.[1];
  if (link && link.includes("/compare/")) parts.push(`**完整变更**：${link}`);

  const footer = resolve(dir, ".github/release-install-notes.md");
  if (existsSync(footer)) parts.push(`---\n\n${readFileSync(footer, "utf8").trim()}`);
  return `${parts.join("\n\n")}\n`;
}
