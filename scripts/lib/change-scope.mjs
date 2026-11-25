// Which checks a set of changed files needs. Anything not listed here counts as
// application code, so a new path is always checked in full until it is added.

/** Only the website reads these; no application build or test does. */
const SITE = /^(website\/|docs\/guides\/|docs\/assets\/screenshots\/)/;

/** Documentation and text that nothing builds. */
const DOCS = /^(docs\/.*\.(md|webp|png|jpg|svg)|[^/]+\.md|LICENSE|\.github\/ISSUE_TEMPLATE\/.*\.md|\.github\/pull_request_template\.md)$/;

/** Everything the website is built from, including files the application shares. */
const SITE_INPUT = /^(website\/|docs\/guides\/|docs\/assets\/screenshots\/|branding\/|frontend\/public\/|CHANGELOG\.md$|package\.json$|bun\.lock$|\.github\/workflows\/(ci|website)\.yml$)/;

/**
 * - `code`: something may affect the application; run everything.
 * - `site`: website files, possibly with documentation; the website checks suffice.
 * - `docs`: documentation only.
 * `siteOnly` is the stricter answer for main: every file belongs to the website.
 * `website` says whether the website has to be built, whatever the scope.
 */
export function changeScope(files) {
  const website = files.some((file) => SITE_INPUT.test(file));
  if (files.length === 0) return { scope: "code", website: true, siteOnly: false };
  const siteOnly = files.every((file) => SITE.test(file));
  if (!files.every((file) => SITE.test(file) || DOCS.test(file))) return { scope: "code", website, siteOnly: false };
  return { scope: files.some((file) => SITE.test(file)) ? "site" : "docs", website, siteOnly };
}

/** Renames are split, so a file moved out of the application still counts as code. */
export function changedFiles(git, range) {
  return git(["diff", "--name-only", "--no-renames", "-z", range]).split("\0").filter(Boolean);
}
