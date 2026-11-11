import { classifyReleaseChange, gitIn } from "./lib/release-pr.mjs";

// bun scripts/check-release-pr.mjs <base> <head>
// Prints why, then `release` or `code` as the last line. CI treats anything other
// than a successful run ending in `release` as a code change.
const [base, head] = process.argv.slice(2);
if (!base || !head) {
  console.error("用法：bun scripts/check-release-pr.mjs <base> <head>");
  process.exit(2);
}
const result = classifyReleaseChange(gitIn(process.cwd()), base, head);
console.log(result.reason);
console.log(result.release ? "release" : "code");
