import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { builtPath, parseArguments, sidecarPath } from "./mcp-sidecar.mjs";

test("sidecar names carry the target triple and the Windows extension", () => {
  expect(sidecarPath("aarch64-apple-darwin", "/b")).toBe("/b/mailx-mcp-aarch64-apple-darwin");
  expect(sidecarPath("x86_64-pc-windows-msvc", "/b")).toBe("/b/mailx-mcp-x86_64-pc-windows-msvc.exe");
});

test("the host triple builds without --target, others land under their triple", () => {
  expect(builtPath("a-linux", "a-linux", true, "/t")).toBe("/t/release/mailx-mcp-bridge");
  expect(builtPath("b-linux", "a-linux", false, "/t")).toBe("/t/b-linux/debug/mailx-mcp-bridge");
  expect(builtPath("x-windows-msvc", "a-linux", true, "/t")).toBe("/t/x-windows-msvc/release/mailx-mcp-bridge.exe");
});

test("Tauri's environment picks the profile and target, flags override it", () => {
  expect(parseArguments([], { TAURI_ENV_DEBUG: "false", TAURI_ENV_TARGET_TRIPLE: "t" })).toEqual({ release: true, target: "t", placeholder: false });
  expect(parseArguments([], { TAURI_ENV_DEBUG: "true" }).release).toBe(false);
  expect(parseArguments(["--debug", "--target", "u", "--placeholder"], { TAURI_ENV_DEBUG: "false" })).toEqual({ release: false, target: "u", placeholder: true });
  expect(() => parseArguments(["--nope"], {})).toThrow();
});

describe("Tauri commands choose the bridge profile explicitly", () => {
  // Tauri CLI 2.12 leaves TAURI_ENV_DEBUG unset for `tauri build`, so without
  // a flag the release installers would carry a debug bridge.
  const config = JSON.parse(readFileSync(join(import.meta.dirname, "../desktop/tauri.conf.json"), "utf8"));

  test("tauri build makes a release bridge", () => {
    expect(config.build.beforeBuildCommand).toContain("scripts/mcp-sidecar.mjs --release");
  });

  test("tauri dev makes a debug bridge", () => {
    expect(config.build.beforeDevCommand).toContain("scripts/mcp-sidecar.mjs --debug");
  });

  test("an unset TAURI_ENV_DEBUG alone still means debug", () => {
    expect(parseArguments([], {}).release).toBe(false);
    expect(parseArguments(["--release"], {}).release).toBe(true);
  });
});
