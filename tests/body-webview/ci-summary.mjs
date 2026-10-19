// Publish synthetic-only evidence through the Checks API as well as artifacts.
// This keeps the failure actionable when the artifact storage is unreachable.
import { readdir, readFile } from 'node:fs/promises';
import { resolve } from 'node:path';

const root = resolve('target/body-webview-evidence');
const escape = (value) => String(value).replaceAll('%', '%25').replaceAll('\r', '%0D').replaceAll('\n', '%0A');
let found = false;
async function visit(directory) {
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = resolve(directory, entry.name);
    if (entry.isDirectory()) await visit(path);
    else if (entry.name === 'report.json') {
      const report = JSON.parse(await readFile(path, 'utf8'));
      if (report.source !== 'synthetic-only') throw new Error('Only synthetic WebView reports may be published.');
      found = true;
      const summary = {
        platform: report.platform, status: report.status, commit: report.commit,
        sourceDirty: report.sourceDirty, binarySha256: report.binarySha256,
        passed: report.assertions.filter((item) => item.passed).length,
        total: report.assertions.length, failure: report.failure,
        failures: report.assertions.filter((item) => !item.passed),
        capabilities: report.capabilities, subscriptionDiagnostics: report.subscriptionDiagnostics, lastObservation: report.lastObservation,
      };
      const severity = report.status === 'passed' ? 'notice' : 'error';
      console.log(`::${severity} title=Native WebView evidence::${escape(JSON.stringify(summary))}`);
      if (report.status !== 'passed') {
        // These reports are produced only by the isolated synthetic fixture app.
        console.log(`::notice title=Native WebView diagnostic::${escape(JSON.stringify({ stages: report.stages, finalState: report.finalState }).slice(0, 24000))}`);
        for (const filename of ['driver.log', 'native-driver.log', 'startup.log']) {
          try {
            const log = await readFile(resolve(directory, filename), 'utf8');
            console.log(`::notice title=Native WebDriver log::${escape(log.slice(-8000))}`);
          } catch (error) { if (error.code !== 'ENOENT') throw error; }
        }
      }
    }
  }
}
try { await visit(root); }
catch (error) { if (error.code !== 'ENOENT') throw error; }
if (!found) console.log('::warning title=Native WebView evidence::No report was produced; the WebView gate has not passed.');
