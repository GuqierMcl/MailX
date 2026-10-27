// Gives every release asset the platform it is built for, so a download is
// chosen by operating system rather than by guessing from an architecture token:
//
//   MailX_0.1.0-alpha.4_macos-arm64.dmg
//   MailX_0.1.0-alpha.4_macos-x64.app.tar.gz
//   MailX_0.1.0-alpha.4_windows-x64-setup.exe
//   MailX_0.1.0-alpha.4_linux-x64.AppImage
//
// The platform label is the release matrix's name. Extensions stay, and a
// `.sig` keeps its whole base name so `updater-manifest.mjs` can pair it with
// exactly one bundle.
//
//   bun scripts/release-assets.mjs --dir dist-release --version 0.1.0-alpha.4 --platform macos-arm64
//
// Only assets the workflow collects are renamed; anything else is left alone.
import { readdirSync, renameSync } from "node:fs";
import { resolve } from "node:path";

/** The release matrix entries, in the order the release publishes them. */
export const PLATFORMS = ["macos-arm64", "macos-x64", "windows-x64", "linux-x64"];

/** Longest first: `.app.tar.gz` must win over any shorter suffix. */
const EXTENSIONS = [".app.tar.gz", "-setup.exe", ".AppImage", ".dmg", ".deb", ".rpm"];

export class UnknownPlatform extends Error {
  constructor(platform) {
    super(`未知的发布平台 "${platform}"：只支持 ${PLATFORMS.join("、")}`);
    this.platform = platform;
  }
}

/**
 * The new name for `name`, or null when the file is not an installer or a
 * signature the release publishes.
 */
export function renamed(name, version, platform) {
  const signature = name.endsWith(".sig");
  const base = signature ? name.slice(0, -".sig".length) : name;
  const extension = EXTENSIONS.find((candidate) => base.endsWith(candidate));
  if (extension === undefined) return null;
  return `MailX_${version}_${platform}${extension}${signature ? ".sig" : ""}`;
}

/** Renames every asset in `directory`; returns the old and new names. */
export function renameAssets(directory, version, platform) {
  if (!PLATFORMS.includes(platform)) throw new UnknownPlatform(platform);
  const changes = [];
  for (const name of readdirSync(directory).sort()) {
    const next = renamed(name, version, platform);
    if (next === null || next === name) continue;
    renameSync(resolve(directory, name), resolve(directory, next));
    changes.push({ from: name, to: next });
  }
  return changes;
}

function argument(name) {
  const index = process.argv.indexOf(`--${name}`);
  return index < 0 ? undefined : process.argv[index + 1];
}

if (import.meta.main) {
  try {
    const version = argument("version");
    const platform = argument("platform");
    if (!version || !platform) {
      throw new Error("用法：--dir <目录> --version <版本> --platform <" + PLATFORMS.join("|") + ">");
    }
    const changes = renameAssets(resolve(argument("dir") ?? "dist-release"), version, platform);
    for (const change of changes) console.log(`${change.from} -> ${change.to}`);
    console.log(`${changes.length} asset(s) named for ${platform}`);
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
