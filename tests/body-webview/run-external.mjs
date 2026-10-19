// Run with the feature-only binary after scripts/body-webview.mjs build.
// This runner never changes a frame sandbox, CSP, or production permission.
import assert from 'node:assert/strict';
import { createHash, randomBytes } from 'node:crypto';
import { createReadStream } from 'node:fs';
import { mkdir, mkdtemp, writeFile } from 'node:fs/promises';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { startExternalDriver } from './external-driver.mjs';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const option = (name, fallback) => process.argv.slice(2).find((argument) => argument.startsWith(`--${name}=`))?.slice(name.length + 3) ?? fallback;
const binary = resolve(option('binary', resolve(root, 'target/debug', process.platform === 'win32' ? 'mailx-desktop.exe' : 'mailx-desktop')));
const output = resolve(option('output', resolve(root, 'target/body-webview-evidence', `${process.platform}-external`)));
const nativeDriver = option('native-driver', undefined);
const tauriDriver = option('tauri-driver', undefined);
const delay = (milliseconds) => new Promise((done) => setTimeout(done, milliseconds));
const hash = createHash('sha256');
for await (const bytes of createReadStream(binary)) hash.update(bytes);
const sourceStatus = spawnSync('git', ['status', '--porcelain=v1'], { cwd: root, encoding: 'utf8' }).stdout;
const trackedDiff = spawnSync('git', ['diff', 'HEAD', '--'], { cwd: root, encoding: 'utf8', maxBuffer: 16 * 1024 * 1024 }).stdout;
await mkdir(output, { recursive: true });
const testRoot = await mkdtemp(resolve(tmpdir(), 'mailx-body-external-'));
await writeFile(resolve(testRoot, '.mailx-synthetic-webview-test'), 'Synthetic body WebView test only.\n');
const canaryRequests = [];
const canary = createServer((request, response) => {
  canaryRequests.push({ method: request.method, path: request.url });
  response.writeHead(200, { 'Content-Type': 'text/plain', 'Access-Control-Allow-Origin': '*' });
  response.end('synthetic canary');
});
await new Promise((done) => canary.listen(0, '127.0.0.1', done));
const report = {
  schemaVersion: 1, startedAt: new Date().toISOString(), platform: process.platform, architecture: process.arch,
  commit: spawnSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).stdout.trim(),
  sourceDirty: sourceStatus.length !== 0, sourceStatus: sourceStatus.trimEnd(),
  trackedDiffSha256: createHash('sha256').update(trackedDiff).digest('hex'),
  binary, binarySha256: hash.digest('hex'), testRoot, source: 'synthetic-only',
  driver: 'official tauri-driver + native platform WebDriver',
  instrumentation: 'Native WebDriver frame selection/execute; unchanged empty sandbox and production CSP.',
  linkInputEvidence: 'W3C native WebDriver element click; no DOM element.click() fallback',
  status: 'running', assertions: [], stages: {}, canaryRequests,
};
let driver;
let logs = '';
function check(name, condition, detail) {
  report.assertions.push({ name, passed: Boolean(condition), ...(detail === undefined ? {} : { detail }) });
  assert.ok(condition, name);
}
async function until(operation, predicate, label, timeout = 30_000) {
  const deadline = Date.now() + timeout;
  let error;
  while (Date.now() < deadline) {
    try { const value = await operation(); if (predicate(value)) return value; } catch (cause) { error = cause; }
    await delay(100);
  }
  throw new Error(`${label} timed out${error ? `: ${error.message}` : ''}`);
}
const frame = (snapshot) => snapshot.observations.at(-1);
const titles = {
  safe: 'MAILX-SAFE-FIXTURE',
  malicious: 'MAILX-MALICIOUS-FIXTURE',
  policyProbe: 'MAILX-POLICY-PROBE',
};
async function screenshot(name) {
  await writeFile(resolve(output, `${name}.png`), Buffer.from(await driver.screenshot(), 'base64'));
}
async function load(name) {
  await driver.execute('return window.__MAILX_BODY_TEST.load(arguments[0]);', name);
  return until(() => driver.observe(), (snapshot) => snapshot.selected === name
    && frame(snapshot).url === snapshot.frameUrl && frame(snapshot).documentReady === 'complete'
    && frame(snapshot).inlineImageLoaded, `${name} native frame observation and CID`);
}
function isolation(name, snapshot) {
  const observed = frame(snapshot);
  check(`${name}: expected fixture loaded`, observed.title === titles[name], observed.title);
  check(`${name}: empty sandbox`, snapshot.sandbox === '');
  check(`${name}: no referrer`, snapshot.referrerPolicy === 'no-referrer');
  check(`${name}: CID image loaded`, observed.inlineImageLoaded && observed.inlineImageWidth > 0);
  check(`${name}: page scripts disabled`, observed.pageScriptExecuted === false);
  check(`${name}: no Tauri globals or invoke`, !observed.tauriGlobalPresent && !observed.tauriInvokePresent);
  check(`${name}: IPC unavailable or denied`, ['unavailable', 'denied'].includes(observed.ipcProbe), observed.ipcProbe);
  check(`${name}: parent DOM and frame DOM isolated`, !observed.parentDomReadable && !snapshot.parentCanReadFrame);
  check(`${name}: parent state and invoke isolated`, !observed.parentStateReadable && !observed.parentInvokeReadable && !snapshot.parentCanReadFrameInternals);
  check(`${name}: parent state unchanged`, snapshot.parentSecret === 'synthetic-parent-state');
  check(`${name}: no remote requests`, canaryRequests.length === 0);
}

try {
  driver = await startExternalDriver({
    binary, cwd: root, nativeDriver, tauriDriver, onLog: (text) => { logs += text; },
    env: { MAILX_BODY_WEBVIEW_ROOT: testRoot, MAILX_BODY_WEBVIEW_NONCE: randomBytes(32).toString('hex'), MAILX_BODY_CANARY_ORIGIN: `http://127.0.0.1:${canary.address().port}` },
  });
  report.capabilities = driver.capabilities;
  await until(() => driver.state(), (snapshot) => snapshot?.ready, 'trusted parent real event subscription');
  const safe = await load('safe');
  report.stages.safe = safe;
  isolation('safe', safe);
  await screenshot('safe-before-link');
  await driver.clickLink();
  await until(() => driver.state(), (snapshot) => snapshot.links.length > safe.links.length, 'real link protocol event');
  await delay(500);
  const linked = await driver.observe();
  report.stages.afterLink = linked;
  check('link: correct database URL', linked.links.at(-1)?.url === 'https://example.invalid/confirmed-link');
  check('link: protocol returned 204', linked.requests.some((request) => request.route === 'link' && request.status === 204));
  check('link: same document marker', frame(linked).documentMarker === frame(safe).documentMarker);
  check('link: document did not navigate', linked.frameUrl === safe.frameUrl && frame(linked).url === frame(safe).url && linked.frameLoads === safe.frameLoads);
  check('link: no remote requests', canaryRequests.length === 0);
  await screenshot('safe-after-link');
  for (const name of ['malicious', 'policyProbe']) {
    await load(name);
    await delay(500);
    const snapshot = await driver.observe();
    report.stages[name] = snapshot;
    isolation(name, snapshot);
    check(`${name}: no forged link request`, snapshot.links.length === linked.links.length);
    check(`${name}: no automatic link protocol request`, snapshot.requests.filter((request) => request.route === 'link').length
      === linked.requests.filter((request) => request.route === 'link').length);
    await screenshot(name);
  }
  report.status = 'passed';
  console.log(`Native WebDriver body checks passed; evidence: ${output}`);
} catch (error) {
  report.status = 'failed';
  report.failure = error.message;
  try { report.finalState = await driver?.state(); if (driver) await screenshot('failure'); } catch { /* record initial failure */ }
  console.error(`Native WebDriver body checks failed: ${error.message}`);
  process.exitCode = 1;
} finally {
  report.finishedAt = new Date().toISOString();
  try {
    await writeFile(resolve(output, 'report.json'), JSON.stringify(report, null, 2));
    await writeFile(resolve(output, 'driver.log'), logs);
  } finally {
    try { await driver?.close(); }
    finally { await new Promise((done) => canary.close(done)); }
  }
}
