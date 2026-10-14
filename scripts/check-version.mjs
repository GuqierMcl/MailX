import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const readJson = (path) => JSON.parse(readFileSync(resolve(root, path), "utf8"));
const rootVersion = readJson("package.json").version;
const frontendVersion = readJson("frontend/package.json").version;
const cargo = readFileSync(resolve(root, "Cargo.toml"), "utf8");
const cargoVersion = cargo.match(/^version = "([^"]+)"/m)?.[1];

const versions = { root: rootVersion, frontend: frontendVersion, cargo: cargoVersion };
const uniqueVersions = new Set(Object.values(versions));
if (!cargoVersion || uniqueVersions.size !== 1) {
  console.error("Version mismatch:", versions);
  process.exit(1);
}

console.log(`MailX version ${rootVersion} (root, frontend, Cargo workspace)`);
