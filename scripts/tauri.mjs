import { spawnSync } from "node:child_process";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const config = resolve(root, "desktop/tauri.conf.json");
const cli = resolve(root, "frontend/node_modules/@tauri-apps/cli/tauri.js");
const args = process.argv.slice(2);
const bun = process.platform === "win32" ? "bun.exe" : "bun";

const result = spawnSync(
  bun,
  [cli, ...args, "--config", config],
  { cwd: root, stdio: "inherit" },
);

if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}

process.exit(result.status ?? 1);
