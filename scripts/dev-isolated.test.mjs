import { Database } from "bun:sqlite";
import { afterEach, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { copyInstalled, dataBase, DEV_DIR_NAME, parseArguments, profiles } from "./dev-isolated.mjs";

const directories = [];
afterEach(() => {
  for (const directory of directories.splice(0)) rmSync(directory, { recursive: true, force: true });
});

function temporary() {
  const directory = mkdtempSync(join(tmpdir(), "mailx-dev-profile-"));
  directories.push(directory);
  return directory;
}

test("uses each platform's application data location", () => {
  expect(dataBase("darwin", {}, "/Users/a")).toBe("/Users/a/Library/Application Support");
  expect(dataBase("win32", { APPDATA: "C:\\Users\\a\\AppData\\Roaming" }, "C:\\Users\\a")).toBe("C:\\Users\\a\\AppData\\Roaming");
  expect(dataBase("linux", {}, "/home/a")).toBe("/home/a/.local/share");
  expect(dataBase("linux", { XDG_DATA_HOME: "/data" }, "/home/a")).toBe("/data");
  expect(dataBase("linux", { XDG_DATA_HOME: "relative" }, "/home/a")).toBe("/home/a/.local/share");
});

test("puts the dev profile next to the installed data unless MAILX_DATA_DIR says otherwise", () => {
  expect(profiles("/base", {})).toEqual({ installed: "/base/MailX", dev: `/base/${DEV_DIR_NAME}` });
  expect(profiles("/base", { MAILX_DATA_DIR: "/elsewhere/p" }).dev).toBe("/elsewhere/p");
  expect(() => profiles("/base", { MAILX_DATA_DIR: "relative" })).toThrow("absolute");
  expect(() => profiles("/base", { MAILX_DATA_DIR: "/base/MailX/" })).toThrow("must not be the installed");
});

test("keeps its own flags and passes the rest to tauri dev", () => {
  expect(parseArguments(["--copy-installed", "--replace", "--copy-only"])).toEqual({ copy: true, replace: true, copyOnly: true, tauri: [] });
  expect(parseArguments(["--release", "--no-watch"]).tauri).toEqual(["--release", "--no-watch"]);
  expect(() => parseArguments(["--replace"])).toThrow("--copy-installed");
  expect(() => parseArguments(["--copy-only"])).toThrow("--copy-installed");
});

function installedProfile() {
  const from = join(temporary(), "MailX");
  mkdirSync(join(from, "blobs", "ab"), { recursive: true });
  mkdirSync(join(from, "mcp"), { recursive: true });
  writeFileSync(join(from, "blobs", "ab", "raw"), "body");
  writeFileSync(join(from, "settings.json"), '{"version":1}');
  writeFileSync(join(from, "mailx.lock"), "");
  writeFileSync(join(from, "mcp", "endpoint"), "x");
  const db = new Database(join(from, "mailx.sqlite3"));
  db.run("PRAGMA journal_mode = WAL");
  db.run("CREATE TABLE notes (text TEXT)");
  db.run("INSERT INTO notes VALUES ('kept in the WAL')");
  // Left open: the newest rows are only in the WAL, as with a running app.
  return { from, db };
}

test("copies a consistent database snapshot and the profile files, not the lock or endpoint", async () => {
  const { from, db } = installedProfile();
  const to = join(temporary(), "MailX-dev");
  await copyInstalled(from, to);
  db.close();
  const copy = new Database(join(to, "mailx.sqlite3"), { readonly: true });
  expect(copy.query("SELECT text FROM notes").all()).toEqual([{ text: "kept in the WAL" }]);
  copy.close();
  expect(readFileSync(join(to, "blobs", "ab", "raw"), "utf8")).toBe("body");
  expect(readFileSync(join(to, "settings.json"), "utf8")).toBe('{"version":1}');
  for (const name of ["mailx.lock", "mcp", "mailx.sqlite3-wal"]) expect(existsSync(join(to, name))).toBe(false);
});

test("never overwrites an existing dev profile unless asked to", async () => {
  const { from, db } = installedProfile();
  db.close();
  const to = join(temporary(), "MailX-dev");
  mkdirSync(to);
  writeFileSync(join(to, "mine"), "keep");
  await expect(copyInstalled(from, to)).rejects.toThrow("--replace");
  expect(readFileSync(join(to, "mine"), "utf8")).toBe("keep");
  await copyInstalled(from, to, { replace: true });
  expect(existsSync(join(to, "mine"))).toBe(false);
  expect(existsSync(join(to, "mailx.sqlite3"))).toBe(true);
});

test("refuses a directory without a MailX database", async () => {
  await expect(copyInstalled(temporary(), join(temporary(), "dev"))).rejects.toThrow("No MailX database");
});
