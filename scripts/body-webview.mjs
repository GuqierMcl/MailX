import assert from 'node:assert/strict';
import { createHash, randomBytes } from 'node:crypto';
import { createServer } from 'node:http';
import { createReadStream } from 'node:fs';
import { mkdtemp, mkdir, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawn, spawnSync } from 'node:child_process';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const args = process.argv.slice(2);
const mode = args[0] ?? 'run';
const option = (name, fallback) => args.find((arg) => arg.startsWith(`--${name}=`))?.slice(name.length + 3) ?? fallback;
const output = resolve(option('output', resolve(root, 'target/body-webview-evidence', process.platform)));
const binary = resolve(option('binary', resolve(root, 'target/debug', process.platform === 'win32' ? 'mailx-desktop.exe' : 'mailx-desktop')));
const inputMode = option('input', 'native');
const fixtureTitles = { safe: 'MAILX-SAFE-FIXTURE', malicious: 'MAILX-MALICIOUS-FIXTURE', policyProbe: 'MAILX-POLICY-PROBE' };
const delay = (milliseconds) => new Promise((done) => setTimeout(done, milliseconds));
const bun = process.platform === 'win32' ? 'bun.exe' : 'bun';

if (mode === 'build') {
  const directory = await mkdtemp(resolve(tmpdir(), 'mailx-body-webview-build-'));
  const configuration = resolve(directory, 'tauri.body-webview.json');
  const original = JSON.parse(await readFile(resolve(root, 'desktop/tauri.conf.json'), 'utf8'));
  // Keep the production CSP unchanged. The implementation commit must have added
  // its real frame-src allowance before this harness can pass.
  await writeFile(configuration, JSON.stringify({
    productName: 'MailX Body WebView Test',
    identifier: 'dev.mailx.body-webview-test',
    // Relative to desktop/, like production's "../frontend/dist". An absolute
    // Windows path such as D:\a\... parses as a URL with scheme "d" (Tauri tries
    // FrontendDist::Url first), so the app opened the folder as a file:// page.
    build: { beforeBuildCommand: null, beforeDevCommand: null, devUrl: null, frontendDist: '../tests/body-webview/assets' },
    app: { security: { csp: original.app.security.csp } },
    bundle: { active: false },
  }));
  const build = spawnSync(bun, ['run', 'tauri', 'build', '--debug', '--no-bundle', '--features', 'body-webview-test', '--config', configuration], { cwd: root, stdio: 'inherit' });
  if (build.error) throw build.error;
  process.exit(build.status ?? 1);
}
if (mode !== 'run' || !['native', 'instrumented', 'manual'].includes(inputMode)) {
  throw new Error('Usage: bun scripts/body-webview.mjs build | run [--input=native|instrumented|manual] [--output=PATH] [--binary=PATH]');
}

if (process.platform !== 'darwin') {
  await import('../tests/body-webview/run-external.mjs');
  process.exit(process.exitCode ?? 0);
}

await mkdir(output, { recursive: true });
const testRoot = await mkdtemp(resolve(tmpdir(), 'mailx-body-webview-data-'));
await writeFile(resolve(testRoot, '.mailx-synthetic-webview-test'), 'Synthetic data only. Safe to remove after the test.\n');
const nonce = randomBytes(32).toString('hex');
const canaryRequests = [];
const canary = createServer((request, response) => {
  canaryRequests.push({ method: request.method, path: request.url });
  response.writeHead(200, { 'Content-Type': 'text/plain', 'Access-Control-Allow-Origin': '*' });
  response.end('synthetic network canary');
});
await new Promise((done) => canary.listen(0, '127.0.0.1', done));
const canaryOrigin = `http://127.0.0.1:${canary.address().port}`;
const reservation = createServer();
await new Promise((done) => reservation.listen(0, '127.0.0.1', done));
const driverPort = reservation.address().port;
await new Promise((done) => reservation.close(done));
const driverOrigin = `http://127.0.0.1:${driverPort}`;
const commit = spawnSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).stdout.trim();
const sourceStatus = spawnSync('git', ['status', '--porcelain=v1'], { cwd: root, encoding: 'utf8' }).stdout;
const trackedDiff = spawnSync('git', ['diff', 'HEAD', '--'], { cwd: root, encoding: 'utf8', maxBuffer: 16 * 1024 * 1024 }).stdout;
const binaryHash = createHash('sha256');
for await (const bytes of createReadStream(binary)) binaryHash.update(bytes);
const harnessSources = Object.fromEntries(await Promise.all([
  'scripts/body-webview.mjs', 'tests/body-webview/native/observer.m', 'tests/body-webview/native/inspect.swift',
  'tests/body-webview/assets/harness.js', 'tests/body-webview/assets/index.html',
].map(async (path) => [path, createHash('sha256').update(await readFile(resolve(root, path))).digest('hex')])));
const report = {
  schemaVersion: 1, startedAt: new Date().toISOString(), platform: process.platform, architecture: process.arch,
  commit, sourceDirty: sourceStatus.length !== 0, sourceStatus: sourceStatus.trimEnd(),
  trackedDiffSha256: createHash('sha256').update(trackedDiff).digest('hex'), binarySha256: binaryHash.digest('hex'),
  harnessSources,
  driver: 'tauri-plugin-wdio-webdriver embedded 1.5.0', inputMode, linkMode: 'numbered-parent',
  source: 'synthetic-only', testRoot, binary, status: 'running', assertions: [], stages: {}, canaryRequests,
  instrumentation: 'Public WKWebView frame evaluation records explicit sandbox denials; Vision OCR and pixel samples verify rendering; AppKit supplies mouse events. Globals blocked by sandbox remain unobservable, not absent.',
  osVersion: spawnSync('sw_vers', ['-productVersion'], { encoding: 'utf8' }).stdout.trim(),
};
let session;
let logs = '';
let child;
let nativeInspector;

async function command(method, path, body) {
  const response = await fetch(`${driverOrigin}${path}`, {
    method, headers: body === undefined ? {} : { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body), signal: AbortSignal.timeout(15_000),
  });
  const result = await response.json();
  if (!response.ok || result.value?.error) throw new Error(`WebDriver ${path}: ${result.value?.error ?? response.status}: ${result.value?.message ?? ''}`);
  return result.value;
}
const execute = (script, ...args) => command('POST', `/session/${session}/execute/sync`, { script, args });
const state = () => execute('return window.__MAILX_BODY_TEST?.state() ?? null;');
async function until(operation, predicate, description, timeout = 20_000) {
  const deadline = Date.now() + timeout;
  let lastError;
  while (Date.now() < deadline) {
    try {
      const value = await operation();
      if (predicate(value)) return value;
    } catch (error) { lastError = error; }
    if (child?.exitCode !== null && child?.exitCode !== undefined) throw new Error('Synthetic test app exited before completion');
    await delay(100);
  }
  throw new Error(`${description} timed out${lastError ? `: ${lastError.message}` : ''}`);
}
function check(name, condition, detail) {
  report.assertions.push({ name, passed: Boolean(condition), ...(detail === undefined ? {} : { detail }) });
}
async function screenshot(name) {
  const encoded = await command('GET', `/session/${session}/screenshot`);
  await writeFile(resolve(output, `${name}.png`), Buffer.from(encoded, 'base64'));
}
async function visualSnapshot(name, snapshot) {
  await screenshot(name);
  const inspected = spawnSync(nativeInspector, [resolve(output, `${name}.png`)], { encoding: 'utf8', timeout: 30_000 });
  if (inspected.status !== 0) throw new Error(`Native screenshot inspection failed: ${inspected.stderr}`);
  return { ...snapshot, visual: JSON.parse(inspected.stdout) };
}
async function waitForFixture(name) {
  await execute('return window.__MAILX_BODY_TEST.load(arguments[0]);', name);
  const loaded = await until(state, (snapshot) => snapshot?.selected === name &&
    snapshot.documentToken && snapshot.bodyLinksLoaded && !snapshot.prepareError && !snapshot.numberedError &&
    snapshot.observations.filter((item) => item.kind === 'native-sandbox-probe' && item.url === snapshot.frameUrl).length === 7 &&
    (name !== 'safe' || snapshot.numberedLinks.length === 1) &&
    snapshot.requests.some((item) => item.route === 'part' && item.status === 200 && item.emailId === snapshot.emailId), `${name} prepared document, CID and native frame probes`);
  return visualSnapshot(name, loaded);
}
function checkMacVisual(name, snapshot) {
  const visual = snapshot.visual;
  check(`${name}: exact rendered fixture title`, visual.recognizedText.some((item) => item.text === fixtureTitles[snapshot.selected] && item.confidence > 0.7));
  const scale = visual.width / snapshot.viewport.width;
  check(`${name}: decoded red CID visible`, visual.redPixelCount > 8_000 * scale ** 2 &&
    Math.abs(visual.redRectangle.width - 96 * scale) <= 2 && Math.abs(visual.redRectangle.height - 96 * scale) <= 2, visual.redRectangle);
  check(`${name}: page attack did not replace fixture marker`, !visual.recognizedText.some((item) => item.text.includes('MAILX-PAGE-SCRIPT-EXECUTED')));
}
function checkIsolation(name, snapshot) {
  checkMacVisual(name, snapshot);
  const documentUrl = new URL(snapshot.frameUrl);
  check(`${name}: document URL carries the prepared binding`, documentUrl.pathname === `/document/${snapshot.emailId}` &&
    documentUrl.searchParams.get('token') === snapshot.documentToken && snapshot.bodyLinksLoaded && !snapshot.prepareError);
  check(`${name}: sandbox remains empty`, snapshot.sandbox === '');
  check(`${name}: no referrer`, snapshot.referrerPolicy === 'no-referrer');
  for (const probe of ['page-script', 'page-globals', 'page-parent-dom', 'page-parent-state', 'page-parent-invoke', 'page-ipc']) {
    const result = snapshot.observations.find((item) => item.url === snapshot.frameUrl && item.probe === probe);
    check(`${name}: ${probe} explicitly blocked by sandbox`, result?.executionBlocked === true, result);
  }
  check(`${name}: globals recorded as unobservable, not absent`, snapshot.observations.some((item) => item.url === snapshot.frameUrl && item.probe === 'page-globals' && item.globalsNotObservable));
  check(`${name}: parent frame access raises SecurityError`, snapshot.parentAccessErrors?.frameDom === 'SecurityError' && snapshot.parentAccessErrors?.frameInternals === 'SecurityError');
  check(`${name}: parent state unchanged`, snapshot.parentSecret === 'synthetic-parent-state');
  check(`${name}: remote request count zero`, canaryRequests.length === 0);
}

async function nativeClick(snapshot) {
  const rectangle = snapshot.numberedLinkRectangle;
  assert.ok(rectangle?.width > 0 && rectangle?.height > 0, 'trusted numbered link button has a visible rectangle');
  const viewportX = rectangle.x + rectangle.width / 2;
  const viewportY = rectangle.y + rectangle.height / 2;
  const target = { x: Math.round(snapshot.screen.x + viewportX), y: Math.round(snapshot.screen.y + viewportY), pid: child.pid };
  report.nativeClickTarget = target;
  if (inputMode === 'instrumented') {
    await execute('window.__MAILX_NATIVE_CLICK_REQUEST={x:arguments[0],y:arguments[1]}; return true;', viewportX, viewportY);
    await until(() => execute('return window.__MAILX_NATIVE_CLICK_DELIVERED===true'), Boolean, 'AppKit native mouse event delivery');
    report.nativeInput = { method: 'public NSWindow.sendEvent mouseDown/mouseUp', x: viewportX, y: viewportY };
    return;
  }
  if (inputMode === 'manual') {
    await writeFile(resolve(output, 'click-ready.json'), JSON.stringify({ target, title: 'MailX Body WebView Test', screenshot: resolve(output, 'safe-before-link.png') }, null, 2));
    console.log(`Waiting for a real click in “MailX Body WebView Test”; target (${target.x}, ${target.y}).`);
    return;
  }
  const native = spawnSync('swift', [resolve(root, 'tests/body-webview/native/click.swift'), String(target.pid), String(target.x), String(target.y)], { encoding: 'utf8', timeout: 30_000 });
  report.nativeInput = { exitCode: native.status, stdout: native.stdout, stderr: native.stderr };
  if (native.error) throw native.error;
  assert.equal(native.status, 0, 'native pointer injection succeeded; no silent synthetic fallback');
}

try {
  const nativeEnvironment = {};
  const helper = resolve(testRoot, 'observer.dylib');
  const compiled = spawnSync('clang', ['-dynamiclib', '-fobjc-arc', '-framework', 'AppKit', '-framework', 'WebKit', '-o', helper, resolve(root, 'tests/body-webview/native/observer.m')], { encoding: 'utf8' });
  if (compiled.status !== 0) throw new Error(`Native observer compile failed: ${compiled.stderr}`);
  nativeEnvironment.DYLD_INSERT_LIBRARIES = helper;
  nativeInspector = resolve(testRoot, 'inspect-screenshot');
  const inspector = spawnSync('swiftc', [resolve(root, 'tests/body-webview/native/inspect.swift'), '-o', nativeInspector], { encoding: 'utf8' });
  if (inspector.status !== 0) throw new Error(`Native image inspector compile failed: ${inspector.stderr}`);
  child = spawn(binary, [], {
    cwd: root, stdio: ['ignore', 'pipe', 'pipe'],
    env: { ...process.env, ...nativeEnvironment, TAURI_WEBDRIVER_PORT: String(driverPort), MAILX_BODY_WEBVIEW_ROOT: testRoot, MAILX_BODY_WEBVIEW_NONCE: nonce, MAILX_BODY_CANARY_ORIGIN: canaryOrigin },
  });
  child.stdout.on('data', (data) => { logs += data; });
  child.stderr.on('data', (data) => { logs += data; });
  child.on('error', (error) => { logs += `spawn error: ${error.message}\n`; });
  await until(() => command('GET', '/status'), (status) => status.ready, 'embedded driver startup', 60_000);
  const created = await command('POST', '/session', { capabilities: { alwaysMatch: { 'wdio:tauriServiceOptions': { windowLabel: 'main' } } } });
  session = created.sessionId;
  report.capabilities = created.capabilities;
  await until(state, (snapshot) => snapshot?.ready, 'trusted test parent and real Tauri event subscription');
  await until(() => execute('return window.__MAILX_NATIVE_OBSERVER_READY===true'), Boolean, 'native frame probe installation');

  const safe = await waitForFixture('safe');
  report.stages.safe = safe;
  checkIsolation('safe', safe);
  check('numbered fallback: trusted list preserves DB URL', safe.numberedLinks[0]?.url === 'https://example.invalid/confirmed-link' && safe.numberedLinks[0]?.index === 0);
  check('numbered fallback: document has passive label', safe.visual.recognizedText.some((item) => item.text === 'Open synthetic link [1]' && item.rectangle.y >= safe.frameRectangle.y * safe.visual.width / safe.viewport.width));
  assert.ok(report.assertions.every((item) => item.passed), 'safe fixture isolation prerequisites');
  await screenshot('safe-before-link');
  await nativeClick(safe);
  await until(state, (snapshot) => snapshot.links.length > safe.links.length && snapshot.numberedRequestSucceeded, 'real numbered-link command and event', inputMode === 'manual' ? 60_000 : 20_000);
  await delay(500);
  const afterLink = await visualSnapshot('safe-after-link', await state());
  report.stages.afterLink = afterLink;
  const requestedLink = afterLink.links.at(-1);
  check('link: correct DB URL emitted', requestedLink?.url === 'https://example.invalid/confirmed-link');
  check('link: exact document binding emitted', requestedLink?.emailId === safe.emailId && requestedLink?.documentToken === safe.documentToken);
  check('link: unique confirmation request emitted', /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/.test(requestedLink?.requestId ?? '') &&
    !safe.links.some((previous) => previous.requestId === requestedLink.requestId));
  check('link: trusted view accepted the exact request', afterLink.currentLinkRequest?.requestId === requestedLink?.requestId &&
    afterLink.currentLinkRequest?.documentToken === safe.documentToken && afterLink.currentLinkRequest?.viewGeneration === safe.viewGeneration &&
    afterLink.acceptedLinks.length === safe.acceptedLinks.length + 1);
  check('link: numbered request succeeded', afterLink.numberedRequestSucceeded && afterLink.numberedError === null);
  check('link: trusted native pointer reached numbered button', afterLink.numberedInput?.trusted === true && afterLink.nativePointer?.target === 'BUTTON');
  check('link: no protocol link navigation requested', !afterLink.requests.some((request) => request.route === 'link'));
  checkMacVisual('link retained document', afterLink);
  check('link: no new document request', afterLink.requests.filter((item) => item.route === 'document').length === safe.requests.filter((item) => item.route === 'document').length);
  check('link: no iframe navigation', afterLink.frameLoads === safe.frameLoads);
  check('link: remote request count zero', canaryRequests.length === 0);
  await screenshot('safe-after-link');

  for (const name of ['malicious', 'policyProbe']) {
    await waitForFixture(name);
    await delay(500);
    const final = await visualSnapshot(name, await state());
    report.stages[name] = final;
    checkIsolation(name, final);
    check(`${name}: previous link confirmation cleared`, final.currentLinkRequest === null);
    check(`${name}: no forged link authorization`, final.links.length === afterLink.links.length);
    check(`${name}: no image request to link route`, !final.requests.some((request) => request.route === 'link'));
    await screenshot(name);
  }
  report.failedAssertions = report.assertions.filter((item) => !item.passed).map((item) => item.name);
  assert.equal(report.failedAssertions.length, 0, `Failed assertions: ${report.failedAssertions.join(', ')}`);
  report.status = 'passed';
  report.linkInputEvidence = inputMode === 'instrumented' ? 'AppKit native mouse events; not physical pointer input' : 'native pointer input';
  console.log(`Body WebView checks passed on ${process.platform}; evidence: ${output}`);
} catch (error) {
  report.status = 'failed';
  report.failure = error.message;
  try { report.finalState = await state(); await screenshot('failure'); } catch {}
  console.error(`Body WebView verification failed: ${error.message}`);
  process.exitCode = 1;
} finally {
  report.finishedAt = new Date().toISOString();
  await writeFile(resolve(output, 'report.json'), JSON.stringify(report, null, 2));
  await writeFile(resolve(output, 'app.log'), logs);
  if (session) { try { await command('DELETE', `/session/${session}`); } catch {} }
  child?.kill();
  await new Promise((done) => canary.close(done));
}
