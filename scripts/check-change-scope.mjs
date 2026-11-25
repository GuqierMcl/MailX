import { changedFiles, changeScope } from "./lib/change-scope.mjs";
import { gitIn } from "./lib/release-pr.mjs";

// bun scripts/check-change-scope.mjs <range>     e.g. base...head
// Prints the files, then `scope=`, `website=` and `site-only=` lines for CI, which
// treats a failed run as a change that needs every check.
const [range] = process.argv.slice(2);
if (!range) {
  console.error("用法：bun scripts/check-change-scope.mjs <range>");
  process.exit(2);
}
const files = changedFiles(gitIn(process.cwd()), range);
for (const file of files) console.log(`  ${JSON.stringify(file)}`);
const { scope, website, siteOnly } = changeScope(files);
console.log(`scope=${scope}\nwebsite=${website}\nsite-only=${siteOnly}`);
