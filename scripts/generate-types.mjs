import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..");
const args = process.argv.slice(2);
if (args.some((arg) => arg !== "--check") || args.length > 1) {
  console.error("Usage: bun scripts/generate-types.mjs [--check]");
  process.exit(1);
}

const check = args.includes("--check");
const target = resolve(root, "frontend/src/generated/accounts.ts");
const startupTarget = resolve(root, "frontend/src/generated/startup.ts");
const result = spawnSync("cargo", ["run", "-p", "mailx-core", "--example", "export_types", "--locked"], {
  cwd: root,
  encoding: "utf8",
  maxBuffer: 8 * 1024 * 1024,
});

if (result.error || result.status !== 0) {
  console.error("Rust DTO generation failed; the existing TypeScript file was not changed.");
  if (result.stderr) process.stderr.write(result.stderr);
  if (result.error) console.error(result.error.message);
  process.exit(1);
}

const generated = result.stdout.replace(/[ \t]+$/gm, "").replace(/\n+$/, "\n");
if (!generated.startsWith("// Generated from Rust") || generated.includes("\r")) {
  console.error("The exporter must produce UTF-8 TypeScript with its generated header and LF line endings.");
  process.exit(1);
}

const startupGenerated = `// Generated from Rust by \`cargo run -p mailx-core --example export_types\`. Do not edit.\n\nexport type { StartupStatus } from './accounts'\n`;

if (check) {
  if (!existsSync(target) || readFileSync(target, "utf8") !== generated ||
      !existsSync(startupTarget) || readFileSync(startupTarget, "utf8") !== startupGenerated) {
    console.error("Rust DTO types have drifted. Run `bun run types:generate` and review the generated changes.");
    process.exit(1);
  }
  console.log("Rust DTO types match frontend/src/generated/accounts.ts.");
} else {
  mkdirSync(dirname(target), { recursive: true });
  writeFileSync(target, generated, "utf8");
  writeFileSync(startupTarget, startupGenerated, "utf8");
  console.log("Generated frontend/src/generated/accounts.ts and startup.ts from Rust DTOs.");
}
