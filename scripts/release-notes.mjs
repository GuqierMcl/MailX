import { releaseNotes } from "./lib/changelog.mjs";
import { checkVersions, repoRoot } from "./lib/version.mjs";

// bun run release:notes [version]   prints the GitHub Release body to stdout
try {
  const version = process.argv[2] ?? checkVersions(repoRoot).version;
  if (!version) throw new Error("仓库里的版本号不一致，无法确定版本");
  process.stdout.write(releaseNotes(repoRoot, version.replace(/^v/, "")));
} catch (error) {
  console.error(error.message);
  process.exit(1);
}
