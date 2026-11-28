import { describe, expect, test } from "bun:test";
import { MACHINES, checkBinaries, peMachine } from "./check-windows-binaries.mjs";

/** A minimal PE header: `MZ`, the PE offset at 0x3c, then `PE\0\0` and the machine. */
function pe(machine) {
  const bytes = Buffer.alloc(0x50);
  bytes.write("MZ", 0, "latin1");
  bytes.writeUInt32LE(0x40, 0x3c);
  bytes.write("PE\0\0", 0x40, "latin1");
  bytes.writeUInt16LE(machine, 0x44);
  return bytes;
}

describe("Windows binary architecture", () => {
  test("reads the COFF machine field", () => {
    expect(peMachine(pe(0xaa64))).toBe(0xaa64);
    expect(peMachine(pe(0x8664))).toBe(0x8664);
    expect(peMachine(Buffer.from("not an executable, just text padding to 64 bytes......."))).toBeNull();
  });

  test("accepts matching executables and names every mismatch", () => {
    const files = { "app.exe": pe(MACHINES["windows-arm64"]), "mcp.exe": pe(MACHINES["windows-arm64"]) };
    expect(() => checkBinaries("windows-arm64", Object.keys(files), (file) => files[file])).not.toThrow();
    const mixed = { "app.exe": pe(MACHINES["windows-arm64"]), "mcp.exe": pe(MACHINES["windows-x64"]) };
    expect(() => checkBinaries("windows-arm64", Object.keys(mixed), (file) => mixed[file])).toThrow(/mcp\.exe：0x8664/);
  });

  test("rejects unknown platforms and an empty file list", () => {
    expect(() => checkBinaries("windows-x86", ["a.exe"], () => pe(0x14c))).toThrow(/未知的 Windows 平台/);
    expect(() => checkBinaries("windows-x64", [], () => pe(0x8664))).toThrow(/没有要检查/);
  });
});
