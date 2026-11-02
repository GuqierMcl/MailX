import { expect, test } from "bun:test";
import { builtPath, parseArguments, sidecarPath } from "./mcp-sidecar.mjs";

test("sidecar names carry the target triple and the Windows extension", () => {
  expect(sidecarPath("aarch64-apple-darwin", "/b")).toBe("/b/mailx-mcp-aarch64-apple-darwin");
  expect(sidecarPath("x86_64-pc-windows-msvc", "/b")).toBe("/b/mailx-mcp-x86_64-pc-windows-msvc.exe");
});

test("the host triple builds without --target, others land under their triple", () => {
  expect(builtPath("a-linux", "a-linux", true, "/t")).toBe("/t/release/mailx-mcp");
  expect(builtPath("b-linux", "a-linux", false, "/t")).toBe("/t/b-linux/debug/mailx-mcp");
  expect(builtPath("x-windows-msvc", "a-linux", true, "/t")).toBe("/t/x-windows-msvc/release/mailx-mcp.exe");
});

test("Tauri's environment picks the profile and target, flags override it", () => {
  expect(parseArguments([], { TAURI_ENV_DEBUG: "false", TAURI_ENV_TARGET_TRIPLE: "t" })).toEqual({ release: true, target: "t", placeholder: false });
  expect(parseArguments([], { TAURI_ENV_DEBUG: "true" }).release).toBe(false);
  expect(parseArguments(["--debug", "--target", "u", "--placeholder"], { TAURI_ENV_DEBUG: "false" })).toEqual({ release: false, target: "u", placeholder: true });
  expect(() => parseArguments(["--nope"], {})).toThrow();
});
