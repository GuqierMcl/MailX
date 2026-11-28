// Checks the CPU type of the Windows executables a release build bundles, so an
// ARM64 installer never ships x64 binaries (or the other way round). The NSIS
// setup stub itself is always x86; only what it installs is checked.
//
//   bun scripts/check-windows-binaries.mjs --platform windows-arm64 \
//     target/release/mailx-desktop.exe desktop/binaries/mailx-mcp-aarch64-pc-windows-msvc.exe
import { readFileSync } from "node:fs";

/** PE / COFF `Machine` values for the release platforms. */
export const MACHINES = { "windows-x64": 0x8664, "windows-arm64": 0xaa64 };

/** The COFF `Machine` field of a PE file, or null when it is not one. */
export function peMachine(bytes) {
  if (bytes.length < 0x40 || bytes[0] !== 0x4d || bytes[1] !== 0x5a) return null;
  const header = bytes.readUInt32LE(0x3c);
  if (header + 6 > bytes.length) return null;
  if (bytes.toString("latin1", header, header + 4) !== "PE\0\0") return null;
  return bytes.readUInt16LE(header + 4);
}

export function checkBinaries(platform, files, read = (file) => readFileSync(file)) {
  const expected = MACHINES[platform];
  if (expected === undefined) throw new Error(`未知的 Windows 平台 "${platform}"：只支持 ${Object.keys(MACHINES).join("、")}`);
  if (files.length === 0) throw new Error("没有要检查的可执行文件");
  const wrong = [];
  for (const file of files) {
    const machine = peMachine(read(file));
    if (machine !== expected) {
      const found = machine === null ? "不是 PE 文件" : `0x${machine.toString(16)}`;
      wrong.push(`${file}：${found}，应为 0x${expected.toString(16)}`);
    }
  }
  if (wrong.length > 0) throw new Error(`架构不符：\n${wrong.join("\n")}`);
}

if (import.meta.main) {
  try {
    const args = process.argv.slice(2);
    const index = args.indexOf("--platform");
    if (index < 0 || !args[index + 1]) throw new Error("用法：--platform <windows-x64|windows-arm64> <exe>...");
    const platform = args[index + 1];
    const files = args.filter((_, position) => position !== index && position !== index + 1);
    checkBinaries(platform, files);
    console.log(`${files.length} executable(s) built for ${platform}`);
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
