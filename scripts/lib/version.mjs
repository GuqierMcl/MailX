// Version rules and the places a MailX version lives in.
//
// The file list is derived, not configured: root package.json plus its
// `workspaces`, the `[workspace.package]` version in Cargo.toml, and the
// Cargo.lock entries of every Cargo workspace member. Adding a member (for
// example `server`) therefore needs no change here as long as it inherits the
// workspace version.
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

export const repoRoot = resolve(import.meta.dirname, "..", "..");
export const PRERELEASE_IDS = ["alpha", "beta", "rc"];

// Stable `x.y.z` or `x.y.z-(alpha|beta|rc).N`. The counter stays within 65535
// so every installer format can carry it.
const VERSION = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-(alpha|beta|rc)\.([1-9]\d*))?$/;
const MAX_COUNTER = 65535;

export function parseVersion(text) {
  const match = VERSION.exec(text);
  if (!match) {
    throw new Error(
      `无效的版本号 "${text}"：只支持 x.y.z 或 x.y.z-(${PRERELEASE_IDS.join("|")}).N（N 从 1 开始）`,
    );
  }
  const pre = match[4] ? { id: match[4], n: Number(match[5]) } : null;
  if (pre && pre.n > MAX_COUNTER) throw new Error(`预发布序号不能超过 ${MAX_COUNTER}：${text}`);
  return { major: Number(match[1]), minor: Number(match[2]), patch: Number(match[3]), pre };
}

export function formatVersion({ major, minor, patch, pre }) {
  return `${major}.${minor}.${patch}${pre ? `-${pre.id}.${pre.n}` : ""}`;
}

export const isPrerelease = (text) => parseVersion(text).pre !== null;

/** SemVer precedence: negative when a < b. A prerelease sorts below its release. */
export function compareVersions(a, b) {
  const x = parseVersion(a);
  const y = parseVersion(b);
  for (const key of ["major", "minor", "patch"]) {
    if (x[key] !== y[key]) return x[key] - y[key];
  }
  if (!x.pre || !y.pre) return (x.pre ? 0 : 1) - (y.pre ? 0 : 1);
  if (x.pre.id !== y.pre.id) {
    return PRERELEASE_IDS.indexOf(x.pre.id) - PRERELEASE_IDS.indexOf(y.pre.id);
  }
  return x.pre.n - y.pre.n;
}

/**
 * The next version for a bump command. Strict on purpose: a stable version is
 * never bumped implicitly out of a prerelease, so `patch` on `1.0.0-rc.2` is an
 * error rather than a surprise `1.0.0`.
 */
export function bumpVersion(current, command, id) {
  const v = parseVersion(current);
  const stable = v.pre === null;
  const needStable = () => {
    if (!stable) {
      throw new Error(`当前版本 ${current} 是预发布版：请用 pre、release 或 set，不能直接 ${command}`);
    }
  };
  const needId = () => {
    if (!PRERELEASE_IDS.includes(id)) {
      throw new Error(`${command} 需要预发布标识：${PRERELEASE_IDS.join(" | ")}`);
    }
  };
  const bump = (kind) => ({
    major: kind === "major" ? v.major + 1 : v.major,
    minor: kind === "major" ? 0 : kind === "minor" ? v.minor + 1 : v.minor,
    patch: kind === "major" || kind === "minor" ? 0 : v.patch + 1,
  });

  let next;
  switch (command) {
    case "major":
    case "minor":
    case "patch":
      needStable();
      next = { ...bump(command), pre: null };
      break;
    case "premajor":
    case "preminor":
    case "prepatch":
      needStable();
      needId();
      next = { ...bump(command.slice(3)), pre: { id, n: 1 } };
      break;
    case "pre": {
      if (stable) {
        throw new Error(`当前版本 ${current} 不是预发布版：请先用 prepatch / preminor / premajor <${PRERELEASE_IDS.join("|")}>`);
      }
      if (id === undefined || id === v.pre.id) {
        next = { ...v, pre: { id: v.pre.id, n: v.pre.n + 1 } };
        break;
      }
      needId();
      if (PRERELEASE_IDS.indexOf(id) < PRERELEASE_IDS.indexOf(v.pre.id)) {
        throw new Error(`不能从 ${v.pre.id} 退回 ${id}；确实需要时用 set --force`);
      }
      next = { ...v, pre: { id, n: 1 } };
      break;
    }
    case "release":
      if (stable) throw new Error(`当前版本 ${current} 已是正式版，没有可去掉的预发布后缀`);
      next = { ...v, pre: null };
      break;
    default:
      throw new Error(`未知的命令 "${command}"`);
  }
  const text = formatVersion(next);
  parseVersion(text); // rejects a counter that overflowed
  return text;
}

const read = (dir, file) => readFileSync(resolve(dir, file), "utf8");
const escapeRegExp = (text) => text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/** The text range of a TOML table body, from its header to the next table. */
function tableRange(text, header) {
  const start = text.search(new RegExp(`^\\[${escapeRegExp(header)}\\][ \\t]*$`, "m"));
  if (start < 0) return null;
  const next = text.slice(start + 1).search(/^\[/m);
  return [start, next < 0 ? text.length : start + 1 + next];
}

function packageFiles(dir) {
  const workspaces = JSON.parse(read(dir, "package.json")).workspaces ?? [];
  return [
    "package.json",
    ...workspaces.map((workspace) => {
      if (workspace.includes("*")) {
        throw new Error(`暂不支持带通配符的 workspaces 条目：${workspace}`);
      }
      return `${workspace}/package.json`;
    }),
  ];
}

function cargoMembers(dir) {
  const list = /^members\s*=\s*\[([^\]]*)\]/m.exec(read(dir, "Cargo.toml"))?.[1];
  if (list === undefined) throw new Error("Cargo.toml 中找不到 workspace members");
  return [...list.matchAll(/"([^"]+)"/g)].map(([, member]) => {
    const manifest = read(dir, `${member}/Cargo.toml`);
    const name = /^name\s*=\s*"([^"]+)"/m.exec(manifest)?.[1];
    if (!name) throw new Error(`${member}/Cargo.toml 缺少 package name`);
    if (!/^version\.workspace\s*=\s*true/m.test(manifest)) {
      throw new Error(`${member}/Cargo.toml 必须使用 version.workspace = true，版本才能统一管理`);
    }
    return { name };
  });
}

// Workspace members have no `source` line in Cargo.lock; registry crates do.
const lockEntry = (name) =>
  new RegExp(`(\\[\\[package\\]\\]\\nname = "${escapeRegExp(name)}"\\nversion = ")([^"]+)("\\n)(?!source = )`);

/** Every version the repository records, in a stable order. */
export function readVersions(dir = repoRoot) {
  const entries = packageFiles(dir).map((file) => ({
    label: file,
    version: JSON.parse(read(dir, file)).version,
  }));
  const cargo = read(dir, "Cargo.toml");
  const range = tableRange(cargo, "workspace.package");
  entries.push({
    label: "Cargo.toml [workspace.package]",
    version: range && /^version\s*=\s*"([^"]+)"/m.exec(cargo.slice(...range))?.[1],
  });
  const lock = read(dir, "Cargo.lock");
  for (const { name } of cargoMembers(dir)) {
    entries.push({ label: `Cargo.lock (${name})`, version: lockEntry(name).exec(lock)?.[2] });
  }
  return entries;
}

export function checkVersions(dir = repoRoot) {
  const entries = readVersions(dir);
  const distinct = new Set(entries.map((entry) => entry.version));
  const ok = distinct.size === 1 && entries.every((entry) => typeof entry.version === "string");
  return { ok, version: ok ? entries[0].version : null, entries };
}

/** New contents (relative path -> text) that set every version to `version`. */
export function planVersionWrite(dir, version) {
  parseVersion(version);
  const files = new Map();
  const replaceOnce = (file, text, pattern, replacement) => {
    if (!pattern.test(text)) throw new Error(`${file}：找不到版本号字段`);
    return text.replace(pattern, replacement);
  };

  for (const file of packageFiles(dir)) {
    files.set(file, replaceOnce(file, read(dir, file), /("version"\s*:\s*")[^"]+(")/, `$1${version}$2`));
  }

  const cargo = read(dir, "Cargo.toml");
  const range = tableRange(cargo, "workspace.package");
  if (!range) throw new Error("Cargo.toml：找不到 [workspace.package]");
  const [start, end] = range;
  files.set(
    "Cargo.toml",
    cargo.slice(0, start) +
      replaceOnce("Cargo.toml", cargo.slice(start, end), /^(version\s*=\s*")[^"]+(")/m, `$1${version}$2`) +
      cargo.slice(end),
  );

  let lock = read(dir, "Cargo.lock");
  for (const { name } of cargoMembers(dir)) {
    lock = replaceOnce(`Cargo.lock (${name})`, lock, lockEntry(name), `$1${version}$3`);
  }
  files.set("Cargo.lock", lock);
  return files;
}

/** `https://github.com/owner/repo` from the root package.json `repository`. */
export function repositoryUrl(dir = repoRoot) {
  const raw = JSON.parse(read(dir, "package.json")).repository;
  const url = typeof raw === "string" ? raw : raw?.url;
  if (!url) throw new Error("package.json 缺少 repository 字段");
  return url.replace(/^git\+/, "").replace(/\.git$/, "");
}
