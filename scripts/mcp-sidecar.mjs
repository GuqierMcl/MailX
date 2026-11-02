// Build the MCP bridge (`mailx-mcp`) and place it where Tauri's
// `bundle.externalBin` expects it: desktop/binaries/mailx-mcp-<target triple>[.exe].
// Tauri needs that file at build time, including `tauri dev` and the build script
// of the desktop crate, so the file must exist before any of them runs.
//
//   bun scripts/mcp-sidecar.mjs [--release | --debug] [--target <triple>]
//   bun scripts/mcp-sidecar.mjs --placeholder   # empty stand-in for cargo check / clippy / test
//
// Defaults follow the Tauri CLI: `TAURI_ENV_TARGET_TRIPLE` and `TAURI_ENV_DEBUG`
// (set for beforeDevCommand / beforeBuildCommand), else the host triple and debug.
// `universal-apple-darwin` builds both macOS architectures and joins them with lipo.
import { spawnSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const PACKAGE = "mailx-mcp-bridge";
const BINARY = "mailx-mcp";
const UNIVERSAL = "universal-apple-darwin";

/** `desktop/binaries/mailx-mcp-<triple>[.exe]`, the name Tauri looks for. */
export function sidecarPath(triple, directory = join(root, "desktop", "binaries")) {
  return join(directory, `${BINARY}-${triple}${triple.includes("windows") ? ".exe" : ""}`);
}

/** Where cargo leaves the bridge for a triple; `host` triples build without `--target`. */
export function builtPath(triple, host, release, targetDirectory = join(root, "target")) {
  const profile = release ? "release" : "debug";
  const file = `${BINARY}${triple.includes("windows") ? ".exe" : ""}`;
  return triple === host ? join(targetDirectory, profile, file) : join(targetDirectory, triple, profile, file);
}

/** Reads `--release`, `--debug`, `--target`, `--placeholder`; the environment fills the gaps. */
export function parseArguments(args, env = process.env) {
  const options = { release: env.TAURI_ENV_DEBUG === "false", target: env.TAURI_ENV_TARGET_TRIPLE || "", placeholder: false };
  for (let index = 0; index < args.length; index += 1) {
    const argument = args[index];
    if (argument === "--release") options.release = true;
    else if (argument === "--debug") options.release = false;
    else if (argument === "--placeholder") options.placeholder = true;
    else if (argument === "--target" && args[index + 1]) options.target = args[++index];
    else throw new Error(`Unknown argument: ${argument}`);
  }
  return options;
}

function run(program, args) {
  const result = spawnSync(program, args, { cwd: root, stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${program} ${args.join(" ")} failed`);
}

function hostTriple() {
  const result = spawnSync("rustc", ["-vV"], { encoding: "utf8" });
  if (result.error) throw result.error;
  const line = result.stdout.split("\n").find((entry) => entry.startsWith("host: "));
  if (!line) throw new Error("rustc -vV did not report a host triple");
  return line.slice("host: ".length).trim();
}

function build(triple, host, release) {
  const args = ["build", "-p", PACKAGE, "--locked"];
  if (release) args.push("--release");
  if (triple !== host) args.push("--target", triple);
  run("cargo", args);
  return builtPath(triple, host, release);
}

export function prepareSidecar(options, host = hostTriple()) {
  const triple = options.target || host;
  const output = sidecarPath(triple);
  mkdirSync(join(root, "desktop", "binaries"), { recursive: true });
  if (options.placeholder) {
    // Enough for the desktop build script, which only checks that the file is there.
    if (!existsSync(output)) writeFileSync(output, "");
    return output;
  }
  if (triple === UNIVERSAL) {
    const parts = ["aarch64-apple-darwin", "x86_64-apple-darwin"].map((part) => build(part, host, options.release));
    run("lipo", ["-create", "-output", output, ...parts]);
  } else {
    copyFileSync(build(triple, host, options.release), output);
  }
  return output;
}

if (import.meta.main) {
  try {
    console.log(`MCP bridge sidecar: ${prepareSidecar(parseArguments(process.argv.slice(2)))}`);
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
