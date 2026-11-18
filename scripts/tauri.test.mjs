import { expect, test } from "bun:test";
import { spawn, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { developmentArgs, developmentBinary, isInformationRequest, validateRunnerEnvironment } from "./tauri.mjs";

test("non-macOS dev arguments pass through without installing a signing runner", () => {
  for (const platform of ["linux", "win32"]) {
    expect(developmentArgs(["--release", "--", "--features", "x"], platform)).toEqual(["dev", "--release", "--", "--features", "x"]);
  }
});

test("both macOS architectures use the signing runner with paths containing spaces", () => {
  const args = developmentArgs([], "darwin", "/runtime with spaces/bun", "/repo with spaces/tauri.mjs");
  const config = (target) => `target.${target}-apple-darwin.runner=["/bin/sh","/repo with spaces/macos-dev-runner.sh","/runtime with spaces/bun","/repo with spaces/tauri.mjs"]`;
  expect(args).toEqual(["dev", "--config", '{"build":{"runner":"cargo"}}', "--",
    "--config", config("aarch64"), "--config", config("x86_64")]);
});

test("Tauri config overrides, Cargo features and application flags keep their meaning", () => {
  const args = developmentArgs(["--config", '{"identifier":"dev.test"}', "--no-watch", "--", "--no-default-features", "--features", "a,b", "--", "--help", "--release"], "darwin");
  expect(args.slice(0, 10)).toEqual(["dev", "--config", '{"identifier":"dev.test"}', "--no-watch", "--config", '{"build":{"runner":"cargo"}}', "--", "--no-default-features", "--features", "a,b"]);
  expect(args.slice(-3)).toEqual(["--", "--help", "--release"]);
  expect(isInformationRequest(["--", "--", "--help"])).toBe(false);
  expect(developmentArgs(["--help"], "darwin")).toEqual(["dev", "--help"]);
});

test("release and custom profiles never enter the debug signing flow", () => {
  for (const args of [["--release"], ["--", "--release"], ["--", "-r"], ["--", "-qr"], ["--", "--profile", "release"], ["--", "--profile=custom"]]) {
    expect(() => developmentArgs(args, "darwin")).toThrow();
  }
});

test("custom Tauri runners and environment overrides cannot silently bypass signing", () => {
  for (const args of [["-r", "custom"], ["-rcustom"], ["-vr", "custom"], ["-er", "custom"], ["-vvercustom"], ["--runner", "custom"], ["--runner=custom"]]) {
    expect(() => developmentArgs(args, "darwin")).toThrow("bypass development signing");
  }
  for (const arch of ["AARCH64", "X86_64"]) {
    expect(() => validateRunnerEnvironment({ [`CARGO_TARGET_${arch}_APPLE_DARWIN_RUNNER`]: "custom" })).toThrow("Unset");
  }
  expect(() => validateRunnerEnvironment({})).not.toThrow();
});

test("only the debug desktop host is eligible, including after resolving symbolic links", () => {
  const directory = realpathSync.native(mkdtempSync(join(tmpdir(), "mailx-runner-")));
  try {
    for (const profile of ["debug", "release"]) {
      mkdirSync(join(directory, profile));
      writeFileSync(join(directory, profile, "mailx-desktop"), "synthetic executable");
    }
    writeFileSync(join(directory, "debug", "mailx-mcp"), "synthetic executable");
    expect(developmentBinary(join(directory, "debug", "mailx-desktop"))).toBe(join(directory, "debug", "mailx-desktop"));
    expect(() => developmentBinary(join(directory, "release", "mailx-desktop"))).toThrow("only accepts");
    expect(() => developmentBinary(join(directory, "debug", "mailx-mcp"))).toThrow("only accepts");
    if (process.platform !== "win32") {
      rmSync(join(directory, "debug", "mailx-desktop"));
      symlinkSync(join(directory, "release", "mailx-desktop"), join(directory, "debug", "mailx-desktop"));
      expect(() => developmentBinary(join(directory, "debug", "mailx-desktop"))).toThrow("only accepts");
    }
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});

const shellTest = process.platform === "darwin" || process.platform === "linux" ? test : test.skip;
const shellRunner = fileURLToPath(new URL("./macos-dev-runner.sh", import.meta.url));

function runnerFixture() {
  const directory = realpathSync.native(mkdtempSync(join(tmpdir(), "mailx runner test ")));
  const runtime = join(directory, "runtime with spaces");
  const signer = join(directory, "fake signer.mjs");
  const binary = join(directory, "native app");
  const signingLog = join(directory, "signing.json");
  const applicationLog = join(directory, "application.json");
  symlinkSync(process.execPath, runtime);
  writeFileSync(signer, [
    'import { writeFileSync } from "node:fs";',
    'writeFileSync(process.env.MAILX_RUNNER_TEST_SIGN_LOG, JSON.stringify(process.argv.slice(2)));',
    'process.exit(Number(process.env.MAILX_RUNNER_TEST_SIGN_EXIT || "0"));',
  ].join("\n"));
  return {
    directory, binary, signingLog, applicationLog,
    args: [shellRunner, runtime, signer, binary],
    env: {
      ...process.env,
      MAILX_RUNNER_TEST_SIGN_LOG: signingLog,
      MAILX_RUNNER_TEST_APP_LOG: applicationLog,
    },
    cleanup: () => rmSync(directory, { recursive: true, force: true }),
  };
}

shellTest("the shell signs first then replaces itself with the app, preserving PID and arguments", () => {
  const fixture = runnerFixture();
  try {
    writeFileSync(fixture.binary, [
      "#!/bin/sh",
      'test -f "$MAILX_RUNNER_TEST_SIGN_LOG" || exit 97',
      'printf \'%s\\n\' "$$"',
      'for argument do printf \'%s\\n\' "$argument"; done',
      "exit 23",
    ].join("\n"), { mode: 0o700 });
    const appArgs = ["argument with spaces", "--", "--flag=value"];
    const result = spawnSync("/bin/sh", [...fixture.args, ...appArgs], { encoding: "utf8", env: fixture.env, timeout: 10000 });
    expect(result.error).toBeUndefined();
    expect(result.status).toBe(23);
    expect(result.stdout.trimEnd().split("\n")).toEqual([String(result.pid), ...appArgs]);
    expect(JSON.parse(readFileSync(fixture.signingLog, "utf8"))).toEqual(["--sign-development", fixture.binary]);
  } finally {
    fixture.cleanup();
  }
});

shellTest("a failed signature stops the runner before launching the app", () => {
  const fixture = runnerFixture();
  try {
    writeFileSync(fixture.binary, '#!/bin/sh\n: > "$MAILX_RUNNER_TEST_APP_LOG"\n', { mode: 0o700 });
    const result = spawnSync("/bin/sh", fixture.args, {
      encoding: "utf8",
      env: { ...fixture.env, MAILX_RUNNER_TEST_SIGN_EXIT: "42" },
      timeout: 10000,
    });
    expect(result.error).toBeUndefined();
    expect(result.status).toBe(42);
    expect(existsSync(fixture.applicationLog)).toBe(false);
    expect(JSON.parse(readFileSync(fixture.signingLog, "utf8"))).toEqual(["--sign-development", fixture.binary]);
  } finally {
    fixture.cleanup();
  }
});

shellTest("SIGTERM reaches the app at the runner PID without leaving a child process", async () => {
  const fixture = runnerFixture();
  let child;
  let applicationPid;
  try {
    // A shell program needs no compiler or actual desktop process. Its builtin
    // loop also avoids adding another child that could obscure an orphan.
    writeFileSync(fixture.binary, [
      "#!/bin/sh",
      'trap \'printf "terminated:%s\\n" "$$"; exit 0\' TERM',
      'printf \'ready:%s\\n\' "$$"',
      "while :; do :; done",
    ].join("\n"), { mode: 0o700 });
    child = spawn("/bin/sh", fixture.args, { env: fixture.env, stdio: ["ignore", "pipe", "pipe"] });
    const closed = new Promise((resolve, reject) => {
      child.once("error", reject);
      child.once("close", (code, signal) => resolve({ code, signal }));
    });
    let output = "";
    await new Promise((resolve, reject) => {
      const deadline = setTimeout(() => reject(new Error("The synthetic app did not become ready.")), 5000);
      child.stdout.on("data", (data) => {
        output += data.toString();
        const match = output.match(/ready:(\d+)\n/);
        if (match) {
          applicationPid = Number(match[1]);
          clearTimeout(deadline);
          resolve();
        }
      });
      child.once("error", (error) => { clearTimeout(deadline); reject(error); });
      child.once("close", () => { clearTimeout(deadline); reject(new Error("The synthetic app exited before readiness.")); });
    });
    expect(applicationPid).toBe(child.pid);
    expect(child.kill("SIGTERM")).toBe(true);
    const deadline = setTimeout(() => child.kill("SIGKILL"), 5000);
    const result = await closed;
    clearTimeout(deadline);
    expect(result).toEqual({ code: 0, signal: null });
    expect(output).toContain(`terminated:${applicationPid}\n`);
    expect(() => process.kill(applicationPid, 0)).toThrow();
  } finally {
    if (child?.exitCode === null && child?.signalCode === null) child.kill("SIGKILL");
    if (applicationPid && applicationPid !== child?.pid) {
      try { process.kill(applicationPid, "SIGKILL"); } catch { /* Already gone. */ }
    }
    fixture.cleanup();
  }
}, 15000);

const macTest = process.platform === "darwin" ? test : test.skip;
macTest("stopping the outer wrapper without a terminal stops its entire subprocess group", async () => {
  const directory = realpathSync.native(mkdtempSync(join(tmpdir(), "mailx process group test ")));
  const wrapper = join(directory, "wrapper.mjs");
  const cli = join(directory, "fake-cli.mjs");
  const grandchild = join(directory, "grandchild.mjs");
  const signals = join(directory, "signals.log");
  let child;
  let cliPid;
  let grandchildPid;
  let deadline;
  try {
    writeFileSync(wrapper, [
      `import { runCommand } from ${JSON.stringify(new URL("./tauri.mjs", import.meta.url).href)};`,
      "runCommand(process.execPath, [process.argv[2], process.argv[3]]);",
    ].join("\n"));
    writeFileSync(cli, [
      'import { spawn } from "node:child_process";',
      'import { appendFileSync } from "node:fs";',
      "let stopping = false;",
      "let childClosed = false;",
      "const finish = () => { if (stopping && childClosed) process.exit(0); };",
      'process.on("SIGTERM", () => {',
      "  stopping = true;",
      '  appendFileSync(process.env.MAILX_RUNNER_TEST_SIGNAL_LOG, `cli:${process.pid}\\n`);',
      "  finish();",
      "});",
      // The fake CLI intentionally never forwards a signal to its child. It
      // waits to reap that child, so killing only the CLI cannot pass this test.
      'const child = spawn(process.execPath, [process.argv[2]], { stdio: "inherit" });',
      'child.on("close", () => { childClosed = true; finish(); });',
      'process.stdout.write(`cli:${process.pid}\\n`);',
    ].join("\n"));
    writeFileSync(grandchild, [
      'import { appendFileSync } from "node:fs";',
      'process.on("SIGTERM", () => {',
      '  appendFileSync(process.env.MAILX_RUNNER_TEST_SIGNAL_LOG, `grandchild:${process.pid}\\n`);',
      "  process.exit(0);",
      "});",
      'process.stdout.write(`grandchild:${process.pid}\\n`);',
      "setInterval(() => {}, 1000);",
    ].join("\n"));
    child = spawn(process.execPath, [wrapper, cli, grandchild], {
      env: { ...process.env, MAILX_RUNNER_TEST_SIGNAL_LOG: signals },
      stdio: ["ignore", "pipe", "pipe"],
    });
    const closed = new Promise((resolve, reject) => {
      child.once("error", reject);
      child.once("close", (code, signal) => resolve({ code, signal }));
    });
    let output = "";
    await new Promise((resolve, reject) => {
      deadline = setTimeout(() => reject(new Error("The synthetic process tree did not become ready.")), 2000);
      child.stdout.on("data", (data) => {
        output += data.toString();
        cliPid = Number(output.match(/cli:(\d+)\n/)?.[1]) || undefined;
        grandchildPid = Number(output.match(/grandchild:(\d+)\n/)?.[1]) || undefined;
        if (cliPid && grandchildPid) {
          clearTimeout(deadline);
          resolve();
        }
      });
      child.once("error", reject);
      child.once("close", () => reject(new Error("The synthetic process tree exited before readiness.")));
    });
    expect(cliPid).not.toBe(child.pid);
    expect(cliPid).not.toBe(process.pid);
    expect(grandchildPid).not.toBe(child.pid);
    expect(child.kill("SIGTERM")).toBe(true);
    const result = await Promise.race([
      closed,
      new Promise((_, reject) => {
        deadline = setTimeout(() => reject(new Error("Stopping the wrapper left a subprocess running.")), 2000);
      }),
    ]);
    clearTimeout(deadline);
    expect(result).toEqual({ code: 0, signal: null });
    const received = readFileSync(signals, "utf8").trim().split("\n").sort();
    expect(received).toEqual([`cli:${cliPid}`, `grandchild:${grandchildPid}`].sort());
    for (const pid of [child.pid, cliPid, grandchildPid]) expect(() => process.kill(pid, 0)).toThrow();
  } finally {
    clearTimeout(deadline);
    if (cliPid && cliPid !== process.pid && cliPid !== child?.pid) {
      try { process.kill(-cliPid, "SIGKILL"); } catch { /* Group already exited. */ }
    }
    // Also clean up individually if a regression prevented group creation.
    for (const pid of [grandchildPid, cliPid, child?.pid]) {
      if (pid && pid !== process.pid) {
        try { process.kill(pid, "SIGKILL"); } catch { /* Already gone. */ }
      }
    }
    rmSync(directory, { recursive: true, force: true });
  }
}, 5000);
