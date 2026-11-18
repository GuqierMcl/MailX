// Development runner adapted from t8y2/dbx scripts/dev-tauri.mjs (Apache-2.0),
// commit eacef66c4ff6fef5bc9a045211435e69d9561c9c; integrated with MailX's CLI.
import { spawn } from "node:child_process";
import { realpathSync } from "node:fs";
import { basename, dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { ensureSigningIdentity, signDevelopmentBinary } from "./macos-dev-signing.mjs";

const entry = fileURLToPath(import.meta.url);
const root = resolve(import.meta.dirname, "..");
const config = resolve(root, "desktop/tauri.conf.json");
const cli = resolve(root, "frontend/node_modules/@tauri-apps/cli/tauri.js");
const targets = ["aarch64-apple-darwin", "x86_64-apple-darwin"];

function splitArguments(args) {
  const separator = args.indexOf("--");
  return separator < 0 ? [args, []] : [args.slice(0, separator), args.slice(separator + 1)];
}

export function isInformationRequest(args) {
  return splitArguments(args)[0].some((arg) => ["--help", "-h", "--version", "-V"].includes(arg));
}

/** Install the Cargo runner on every watch rebuild, preserving all three argument levels. */
export function developmentArgs(args, platform = process.platform, executable = process.execPath, runner = entry) {
  if (platform !== "darwin" || isInformationRequest(args)) return ["dev", ...args];
  const [tauriArgs, cargoArgs] = splitArguments(args);
  const [buildArgs, appArgs] = splitArguments(cargoArgs);
  if (tauriArgs.includes("--release") || buildArgs.some((arg) => arg === "--release" || /^-[qv]*r/.test(arg))) {
    throw new Error("Use tauri build for release builds; development signing only supports debug builds.");
  }
  if (buildArgs.some((arg) => arg === "--profile" || arg.startsWith("--profile="))) {
    throw new Error("Custom Cargo profiles are not supported by development signing; use the default debug profile.");
  }
  if (tauriArgs.some((arg) => /^-[vehV]*r/.test(arg) || arg === "--runner" || arg.startsWith("--runner="))) {
    throw new Error("Custom Tauri runners bypass development signing; use the default Cargo runner.");
  }
  // Cargo/Tauri kill the runner itself when rebuilding. The shell must exec the
  // app after signing, so no launcher survives between Tauri and the native PID.
  const runnerValue = JSON.stringify(["/bin/sh", resolve(dirname(runner), "macos-dev-runner.sh"), executable, runner]);
  const runnerArgs = targets.flatMap((target) => ["--config", `target.${target}.runner=${runnerValue}`]);
  // A config file may set build.runner too. Ensure Cargo consumes the target runner.
  return ["dev", ...tauriArgs, "--config", JSON.stringify({ build: { runner: "cargo" } }), "--", ...buildArgs, ...runnerArgs,
    ...(cargoArgs.includes("--") ? ["--", ...appArgs] : [])];
}

export function validateRunnerEnvironment(env = process.env) {
  for (const target of targets) {
    const variable = `CARGO_TARGET_${target.toUpperCase().replaceAll("-", "_")}_RUNNER`;
    if (env[variable]) throw new Error(`Unset ${variable} before using the MailX development signing runner.`);
  }
}

export function developmentBinary(candidate) {
  const binary = realpathSync.native(candidate);
  if (basename(binary) !== "mailx-desktop" || basename(dirname(binary)) !== "debug") {
    throw new Error("The development signing runner only accepts a debug/mailx-desktop executable.");
  }
  return binary;
}

export function runCommand(executable, args, options = {}) {
  // Give the macOS CLI its own process group. An agent stopping this wrapper
  // must also stop Cargo, the native app and Vite, even without a terminal.
  const ownGroup = process.platform === "darwin";
  const child = spawn(executable, args, { cwd: root, stdio: "inherit", env: process.env, ...options, detached: ownGroup });
  const handlers = new Map();
  for (const signal of ["SIGINT", "SIGTERM"]) {
    const handler = () => {
      try {
        if (ownGroup && child.pid) process.kill(-child.pid, signal);
        else child.kill(signal);
      } catch (error) {
        if (error.code !== "ESRCH") throw error;
      }
    };
    handlers.set(signal, handler);
    process.on(signal, handler);
  }
  child.on("error", (error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
  child.on("close", (code, signal) => {
    for (const [name, handler] of handlers) process.removeListener(name, handler);
    if (signal) process.kill(process.pid, signal);
    else process.exitCode = code ?? 1;
  });
}

export function main(args = process.argv.slice(2)) {
  if (args[0] === "--sign-development") {
    if (process.platform !== "darwin" || !args[1]) throw new Error("The signing runner requires a macOS development executable.");
    const binary = developmentBinary(args[1]);
    signDevelopmentBinary(binary);
    return;
  }
  let cliArgs = args;
  if (args[0] === "dev") {
    cliArgs = developmentArgs(args.slice(1));
    if (process.platform === "darwin" && !isInformationRequest(args.slice(1))) {
      validateRunnerEnvironment();
      ensureSigningIdentity();
    }
  }
  // Load the repository config first so explicit CLI overrides take precedence.
  runCommand(process.execPath, cliArgs.length ? [cli, cliArgs[0], "--config", config, ...cliArgs.slice(1)] : [cli, "--help"]);
}

if (import.meta.main) {
  try {
    main();
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
