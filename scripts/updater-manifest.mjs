// Builds the `latest.json` the in-app updater reads (docs/architecture/updater.md §4).
//
// One entry per desktop platform the release supports. Every one is required: a
// manifest missing a platform would silently strand those users on an old
// version, so a missing bundle fails the release instead.
//
// Usage (the release workflow calls it):
//   bun scripts/updater-manifest.mjs --version 0.2.0 --dir dist \
//     --notes release-notes.md --base-url https://…/releases/download/v0.2.0 \
//     --pub-date 2026-11-01T08:00:00Z --out latest.json
import { readFileSync, readdirSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { sectionBody } from "./lib/changelog.mjs";

/** The manifest key, and how an asset file belongs to it. */
export const PLATFORMS = [
  // Tauri names the macOS bundle after the build's architecture (`aarch64`,
  // `x64`); the workflow renames it so the two never collide.
  { key: "darwin-aarch64", match: (name) => name.endsWith(".app.tar.gz") && /(^|[_.-])(aarch64|arm64)([_.-]|$)/.test(name) },
  { key: "darwin-x86_64", match: (name) => name.endsWith(".app.tar.gz") && /(^|[_.-])(x86_64|x64)([_.-]|$)/.test(name) },
  { key: "windows-x86_64", match: (name) => name.endsWith("-setup.exe") },
  { key: "linux-x86_64", match: (name) => name.endsWith(".AppImage") },
];

export class MissingPlatforms extends Error {
  constructor(keys) {
    super(`更新清单缺少平台：${keys.join("、")}`);
    this.keys = keys;
  }
}

/**
 * The CHANGELOG section as updater notes: no "预发布版本" banner and no install
 * footer, since the app shows them beside its own buttons.
 */
export function notesForUpdater(changelog, version) {
  const name = /-(alpha|beta|rc)\./.test(version) ? "Unreleased" : version;
  const body = sectionBody(changelog, name);
  if (body === null) return null;
  const lines = body.split("\n");
  const kept = [];
  for (const line of lines) {
    if (/^> \*\*预发布版本\*\*/.test(line.trim())) continue;
    // The install footer starts at a horizontal rule; nothing after it is a note.
    if (/^---\s*$/.test(line.trim())) break;
    kept.push(line);
  }
  const text = kept.join("\n").trim();
  return text.length > 0 ? text : null;
}

/**
 * `files` are asset names that will sit next to `latest.json`. `readSignature`
 * returns a `.sig` file's text. Throws `MissingPlatforms` for anything absent.
 */
export function buildManifest({ version, notes, pubDate, baseUrl, files, readSignature }) {
  const present = new Set(files);
  const platforms = {};
  const missing = [];
  for (const platform of PLATFORMS) {
    const asset = files.find((name) => !name.endsWith(".sig") && platform.match(name));
    if (!asset || !present.has(`${asset}.sig`)) {
      missing.push(platform.key);
      continue;
    }
    const signature = readSignature(`${asset}.sig`).trim();
    if (!signature) {
      missing.push(platform.key);
      continue;
    }
    platforms[platform.key] = {
      signature,
      url: `${baseUrl}/${encodeURIComponent(asset)}`,
    };
  }
  if (missing.length > 0) throw new MissingPlatforms(missing);
  return { version, notes: notes ?? "", pub_date: pubDate, platforms };
}

function argument(name) {
  const index = process.argv.indexOf(`--${name}`);
  return index < 0 ? undefined : process.argv[index + 1];
}

if (import.meta.main) {
  try {
    const version = argument("version");
    const directory = resolve(argument("dir") ?? "dist");
    const baseUrl = argument("base-url");
    const pubDate = argument("pub-date") ?? new Date().toISOString();
    const notesFile = argument("notes");
    if (!version || !baseUrl || !argument("dir")) {
      throw new Error("用法：--version <版本> --dir <目录> --base-url <下载地址前缀> [--pub-date <ISO 时间>] [--notes <发布说明文件>] [--out <输出文件>]");
    }
    const noteText = notesFile
      ? notesForUpdater(readFileSync(resolve(notesFile), "utf8"), version)
      : null;
    const manifest = buildManifest({
      version,
      notes: noteText,
      pubDate,
      baseUrl,
      files: readdirSync(directory),
      readSignature: (name) => readFileSync(resolve(directory, name), "utf8"),
    });
    writeFileSync(resolve(argument("out") ?? resolve(directory, "latest.json")), `${JSON.stringify(manifest, null, 2)}\n`);
    console.log(`latest.json: ${version}, ${Object.keys(manifest.platforms).length} platforms`);
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
