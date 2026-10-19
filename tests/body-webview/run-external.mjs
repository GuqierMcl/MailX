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
const numberedFallback = process.platform === 'linux';
const requestIdPattern = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
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
  driver: process.platform === 'win32' ? 'Microsoft Edge WebDriver (WebView2)' : 'official tauri-driver + WebKitWebDriver',
  instrumentation: 'Native WebDriver frame selection/execute; unchanged empty sandbox and production CSP.',
  linkInputEvidence: numberedFallback
    ? 'W3C native WebDriver element click on trusted parent numbered button; no DOM element.click() fallback'
    : 'W3C native WebDriver element click in the body frame; no DOM element.click() fallback',
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
  return until(() => driver.observe(), (snapshot) => {
    const observed = frame(snapshot);
    if (!snapshot || !observed || snapshot.selected !== name || !snapshot.emailId || !snapshot.documentToken
      || !snapshot.bodyLinksLoaded || snapshot.prepareError || snapshot.numberedError
      || observed.url !== snapshot.frameUrl || observed.documentReady !== 'complete' || !observed.inlineImageLoaded) {
      return false;
    }
    if (!Array.isArray(snapshot.numberedLinks)) return false;
    let documentUrl;
    try { documentUrl = new URL(snapshot.frameUrl); } catch { return false; }
    return documentUrl.pathname === `/document/${snapshot.emailId}`
      && documentUrl.searchParams.get('token') === snapshot.documentToken
      && snapshot.numberedLinks.length === (numberedFallback && name !== 'malicious' ? 1 : 0);
  }, `${name} bound document, native frame observation and CID`);
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
    binary, cwd: root, nativeDriver, tauriDriver, nativeLog: resolve(output, 'native-driver.log'), onLog: (text) => { logs += text; },
    env: { MAILX_BODY_WEBVIEW_ROOT: testRoot, MAILX_BODY_WEBVIEW_NONCE: randomBytes(32).toString('hex'), MAILX_BODY_CANARY_ORIGIN: `http://127.0.0.1:${canary.address().port}` },
  });
  report.capabilities = driver.capabilities;
  await until(() => driver.state(), (snapshot) => snapshot?.ready, 'trusted parent real event subscription');
  const safe = await load('safe');
  report.stages.safe = safe;
  isolation('safe', safe);
  if (numberedFallback) {
    check('numbered fallback: trusted list preserves DB URL', safe.numberedLinks.length === 1
      && safe.numberedLinks[0]?.emailId === safe.emailId
      && safe.numberedLinks[0]?.index === 0
      && safe.numberedLinks[0]?.url === 'https://example.invalid/confirmed-link');
    check('numbered fallback: trusted parent button is visible', safe.numberedLinkRectangle?.width > 0
      && safe.numberedLinkRectangle?.height > 0, safe.numberedLinkRectangle);
  }
  await screenshot('safe-before-link');
  if (numberedFallback) await driver.clickNumberedLink();
  else await driver.clickLink();
  await until(() => driver.state(), (snapshot) => snapshot.links.length > safe.links.length
    && (numberedFallback ? snapshot.numberedRequestSucceeded : true), 'real link event');
  await delay(500);
  const linked = await driver.observe();
  report.stages.afterLink = linked;
  const requestedLink = linked.links.at(-1);
  check('link: correct database URL', requestedLink?.url === 'https://example.invalid/confirmed-link');
  check('link: exact email binding emitted', requestedLink?.emailId === safe.emailId);
  check('link: exact document token emitted', requestedLink?.documentToken === safe.documentToken);
  check('link: unique request ID emitted', requestIdPattern.test(requestedLink?.requestId ?? '')
    && !safe.links.some((previous) => previous.requestId === requestedLink.requestId));
  check('link: trusted view accepted exact request', linked.currentLinkRequest?.requestId === requestedLink?.requestId
    && linked.currentLinkRequest?.documentToken === safe.documentToken
    && linked.currentLinkRequest?.emailId === safe.emailId);
  if (numberedFallback) {
    check('link: numbered request succeeded', linked.numberedRequestSucceeded && linked.numberedError === null);
    check('link: no link protocol request', !linked.requests.some((request) => request.route === 'link'));
  } else {
    check('link: protocol returned 204', linked.requests.some((request) => request.route === 'link' && request.status === 204));
  }
  check('link: same document marker', frame(linked).documentMarker === frame(safe).documentMarker);
  check('link: document did not navigate', linked.frameUrl === safe.frameUrl && frame(linked).url === frame(safe).url && linked.frameLoads === safe.frameLoads);
  check('link: title retained', frame(linked).title === titles.safe);
  check('link: CID retained', frame(linked).inlineImageLoaded && frame(linked).inlineImageWidth > 0);
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
