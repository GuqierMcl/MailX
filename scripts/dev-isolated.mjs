// Run the desktop app in development mode with its own profile, so it can run
// next to an installed MailX without sharing its data, passwords or instance.
//
//   bun run desktop:dev:isolated                          # start with the dev profile
//   bun run desktop:dev:isolated -- --copy-installed      # copy the installed data first
//   bun run desktop:dev:isolated -- --copy-installed --replace --copy-only
//
// The profile directory is `MAILX_DATA_DIR` when set, else `MailX-dev` next to
// the installed `MailX` directory. The app derives a separate credential service
// (`MailX.<folder>`) from it, and the dev identifier below gives it its own
// single-instance lock, WebView storage and window state.
//
// `--copy-installed` copies the database (a consistent snapshot, even while the
// installed app runs), settings, downloaded bodies and the image cache. Passwords
// and keys are not copied: they stay in the system credential store under the
// installed profile's service. Any extra arguments go to `tauri dev`.
import { spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readdirSync, rmSync } from "node:fs";
import { homedir } from "node:os";
import { isAbsolute, join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");

/** Folder of the installed app's data, as in `desktop/src/storage_location.rs`. */
export const INSTALLED_DIR_NAME = "MailX";
export const DEV_DIR_NAME = "MailX-dev";
export const DEV_IDENTIFIER = "com.mailxapp.desktop.dev";
const OVERRIDE = "MAILX_DATA_DIR";

/** The parent of Tauri's `app_data_dir()` on each platform. */
export function dataBase(platform = process.platform, env = process.env, home = homedir()) {
  if (platform === "darwin") return join(home, "Library", "Application Support");
  if (platform === "win32") {
    if (!env.APPDATA) throw new Error("APPDATA is not set");
    return env.APPDATA;
  }
  return env.XDG_DATA_HOME && isAbsolute(env.XDG_DATA_HOME) ? env.XDG_DATA_HOME : join(home, ".local", "share");
}

/** Where the installed app keeps its data and where the dev profile goes. */
export function profiles(base = dataBase(), env = process.env) {
  const installed = join(base, INSTALLED_DIR_NAME);
  const dev = env[OVERRIDE] || join(base, DEV_DIR_NAME);
  if (!isAbsolute(dev)) throw new Error(`${OVERRIDE} must be an absolute path`);
  if (resolve(dev) === resolve(installed)) {
    throw new Error(`The dev profile must not be the installed data directory: ${installed}`);
  }
  return { installed, dev };
}

/** Reads the script's own flags; everything else is passed to `tauri dev`. */
export function parseArguments(args) {
  const options = { copy: false, replace: false, copyOnly: false, tauri: [] };
  for (const argument of args) {
    if (argument === "--copy-installed") options.copy = true;
    else if (argument === "--replace") options.replace = true;
    else if (argument === "--copy-only") options.copyOnly = true;
    else options.tauri.push(argument);
  }
  if ((options.replace || options.copyOnly) && !options.copy) {
    throw new Error("--replace and --copy-only only apply together with --copy-installed");
  }
  return options;
}

/** Copied besides the database. The lock, MCP endpoint files and window state are per profile. */
const COPIED = ["settings.json", "blobs", "image-cache"];

/**
 * Copies the installed data into `to`. The database is written with `VACUUM INTO`,
 * which reads one consistent snapshot even while another process writes to it.
 */
export async function copyInstalled(from, to, { replace = false } = {}) {
  if (!existsSync(join(from, "mailx.sqlite3"))) throw new Error(`No MailX database in ${from}`);
  if (existsSync(to) && readdirSync(to).length > 0) {
    if (!replace) throw new Error(`${to} is not empty; add --replace to overwrite it`);
    rmSync(to, { recursive: true, force: true });
  }
  mkdirSync(to, { recursive: true });
  const { Database } = await import("bun:sqlite");
  const source = new Database(join(from, "mailx.sqlite3"), { readonly: true });
  try {
    source.run("VACUUM INTO ?", [join(to, "mailx.sqlite3")]);
  } finally {
    source.close();
  }
  for (const name of COPIED) {
    const path = join(from, name);
    if (existsSync(path)) cpSync(path, join(to, name), { recursive: true });
  }
}

function launch(dev, args) {
  const config = JSON.stringify({ identifier: DEV_IDENTIFIER });
  const bun = process.platform === "win32" ? "bun.exe" : "bun";
  const result = spawnSync(bun, [join(root, "scripts", "tauri.mjs"), "dev", "--config", config, ...args], {
    cwd: root,
    stdio: "inherit",
    env: { ...process.env, [OVERRIDE]: dev },
  });
  if (result.error) throw result.error;
  return result.status ?? 1;
}

if (import.meta.main) {
  try {
    const options = parseArguments(process.argv.slice(2));
    const { installed, dev } = profiles();
    if (options.copy) {
      await copyInstalled(installed, dev, options);
      console.log(`Copied the installed data from ${installed} to ${dev}.`);
      console.log("Passwords and keys were not copied; enter them again in the dev profile.");
    }
    if (!options.copyOnly) {
      console.log(`Dev profile: ${dev} (identifier ${DEV_IDENTIFIER})`);
      process.exit(launch(dev, options.tauri));
    }
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
