import { expect, test } from "bun:test";
import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { fileURLToPath } from "node:url";
import { designatedRequirement, ensureSigningIdentity, loadSigningIdentity, removeSigningKeychain, signDevelopmentBinary } from "./macos-dev-signing.mjs";

function temporary(run) {
  const directory = realpathSync.native(mkdtempSync(join(tmpdir(), "mailx-dev-signing-test-")));
  try {
    return run(directory);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

test("the development identity binds the identifier and certificate, rather than a build hash", () => {
  const fingerprint = "A1".repeat(20);
  expect(designatedRequirement(fingerprint)).toBe(`identifier "com.mailxapp.desktop.dev" and certificate leaf = H"${fingerprint}"`);
  for (const invalid of ["", "-", "A1", "a1".repeat(20), '" or true']) {
    expect(() => designatedRequirement(invalid)).toThrow();
  }
});

const macTest = process.platform === "darwin" ? test : test.skip;

macTest("an interrupted setup preserves the existing identity instead of creating a replacement", () => {
  temporary((directory) => {
    const keychain = join(directory, "development.keychain-db");
    writeFileSync(keychain, "existing identity", { mode: 0o600 });
    expect(() => ensureSigningIdentity(directory)).toThrow("Incomplete development signing");
    expect(readFileSync(keychain, "utf8")).toBe("existing identity");
    expect(existsSync(join(directory, "identity.json"))).toBe(false);
    expect(existsSync(join(directory, "setup.lock"))).toBe(false);
  });
});

macTest("a corrupt published identity is retained and never silently regenerated", () => {
  temporary((directory) => {
    const files = {
      "identity.json": "{invalid configuration",
      "certificate.pem": "existing certificate",
      "development.keychain-db": "existing keychain",
    };
    for (const [name, content] of Object.entries(files)) writeFileSync(join(directory, name), content, { mode: 0o600 });
    expect(() => ensureSigningIdentity(directory)).toThrow("Invalid development signing");
    for (const [name, content] of Object.entries(files)) expect(readFileSync(join(directory, name), "utf8")).toBe(content);
  });
});

macTest("an existing setup lock prevents a concurrent initializer from replacing files", () => {
  temporary((directory) => {
    const lock = join(directory, "setup.lock");
    writeFileSync(lock, "another setup", { mode: 0o600 });
    expect(() => ensureSigningIdentity(directory)).toThrow();
    expect(readFileSync(lock, "utf8")).toBe("another setup");
    expect(existsSync(join(directory, "identity.json"))).toBe(false);
  });
});

macTest("signing refuses symbolic links before reading or creating a keychain", () => {
  temporary((directory) => {
    const real = join(directory, "real");
    const linked = join(directory, "linked");
    mkdirSync(real, { mode: 0o700 });
    symlinkSync(real, linked);
    expect(() => loadSigningIdentity(linked)).toThrow("unsafe path");
    expect(() => ensureSigningIdentity(linked)).toThrow("unsafe path");
    const external = join(directory, "outside.json");
    writeFileSync(external, "do not read or replace", { mode: 0o600 });
    symlinkSync(external, join(real, "identity.json"));
    expect(() => ensureSigningIdentity(real)).toThrow("unsafe path");
    expect(readFileSync(external, "utf8")).toBe("do not read or replace");
  });
});

// This probe creates only a temporary signing keychain and a synthetic password
// inside it. It never reads MailX credentials or the default login keychain.
const keychainTest = process.platform === "darwin" && process.env.MAILX_TEST_MACOS_KEYCHAIN === "1" ? test : test.skip;
keychainTest("different builds retain Keychain access without user interaction", () => {
  temporary((directory) => {
    const signing = join(directory, "signing");
    let identity;
    const execute = (command, args) => {
      try {
        return execFileSync(command, args, {
          encoding: "utf8",
          stdio: ["ignore", "pipe", "pipe"],
          timeout: 30000,
        });
      } catch (error) {
        // Even this temporary keychain's password should stay out of test logs.
        throw new Error(`Synthetic Keychain probe: ${basename(command)} failed (${error.status ?? error.code}).`);
      }
    };
    try {
      identity = ensureSigningIdentity(signing);
      const binary = join(directory, "probe");
      const source = fileURLToPath(new URL("./fixtures/macos-dev-keychain.c", import.meta.url));
      const compile = (version) => execute("/usr/bin/clang", [
        "-Wno-deprecated-declarations", `-DMAILX_PROBE_VERSION=${version}`, source,
        "-framework", "Security", "-framework", "CoreFoundation", "-o", binary,
      ]);
      const signature = () => {
        const result = spawnSync("/usr/bin/codesign", ["-d", "-r-", "--verbose=4", binary], { encoding: "utf8", timeout: 10000 });
        expect(result.error).toBeUndefined();
        expect(result.status).toBe(0);
        const output = result.stdout + result.stderr;
        return { requirement: output.match(/designated => (.+)/)?.[1], hash: output.match(/^CDHash=(.+)$/m)?.[1] };
      };
      const probe = () => execute(binary, [identity.keychain, "read"]);
      compile(1);
      signDevelopmentBinary(binary, signing);
      const first = signature();
      expect(execute(binary, [identity.keychain, "create"])).toContain("build=1 status=0");
      expect(first.requirement).toBeTruthy();
      expect(first.hash).toBeTruthy();

      compile(2);
      const unsigned = spawnSync(binary, [identity.keychain, "read"], { encoding: "utf8", timeout: 5000 });
      expect(unsigned.error).toBeUndefined();
      expect(unsigned.status).toBe(3);
      signDevelopmentBinary(binary, signing);
      const second = signature();
      expect(second.hash).not.toBe(first.hash);
      expect(second.requirement).toBe(first.requirement);
      expect(probe()).toContain("build=2 status=0");

      compile(3);
      signDevelopmentBinary(binary, signing);
      const third = signature();
      expect(third.hash).not.toBe(second.hash);
      expect(third.requirement).toBe(first.requirement);
      expect(probe()).toContain("build=3 status=0");
      expect(ensureSigningIdentity(signing).fingerprint).toBe(identity.fingerprint);
      const searchList = execute("/usr/bin/security", ["list-keychains", "-d", "user"]);
      expect([...searchList.matchAll(/"([^"\n]+)"/g)].filter((match) => match[1] === identity.keychain)).toHaveLength(1);

      execute("/usr/bin/security", ["lock-keychain", identity.keychain]);
      const locked = spawnSync(binary, [identity.keychain, "read"], { encoding: "utf8", timeout: 5000 });
      expect(locked.error).toBeUndefined();
      expect(locked.status).toBe(3);
      expect(locked.stdout).toMatch(/status=-\d+/);
      execute("/usr/bin/security", ["unlock-keychain", "-p", identity.password, identity.keychain]);
      expect(probe()).toContain("build=3 status=0");
    } finally {
      if (identity) removeSigningKeychain(identity.keychain);
    }
  });
}, 120000);
