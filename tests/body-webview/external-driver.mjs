// Standard native WebDriver adapter for Linux WebKitGTK and Windows WebView2.
// This is test instrumentation: frame selection happens in the native driver,
// never by reading parent.contentWindow or changing an iframe's sandbox/CSP.
import { access } from 'node:fs/promises';
import { createServer } from 'node:net';
import { isAbsolute, resolve } from 'node:path';
import { spawn } from 'node:child_process';

const ELEMENT = 'element-6066-11e4-a52e-4f735466cecf';
const delay = (milliseconds) => new Promise((done) => setTimeout(done, milliseconds));

async function freePort() {
  const server = createServer();
  await new Promise((done, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', done); });
  const port = server.address().port;
  await new Promise((done) => server.close(done));
  return port;
}

export async function startExternalDriver({ binary, cwd, env, tauriDriver, nativeDriver, nativeLog, onLog = () => {} }) {
  if (!['linux', 'win32'].includes(process.platform)) {
    throw new Error('The official external tauri-driver supports Linux and Windows only.');
  }
  if (!isAbsolute(binary) || !env?.MAILX_BODY_WEBVIEW_ROOT || !isAbsolute(env.MAILX_BODY_WEBVIEW_ROOT)) {
    throw new Error('External driver requires an absolute synthetic binary and isolated test root.');
  }
  await access(binary);
  await access(resolve(env.MAILX_BODY_WEBVIEW_ROOT, '.mailx-synthetic-webview-test'));
  const [port, nativePort, embeddedPort] = await Promise.all([freePort(), freePort(), freePort()]);
  if (new Set([port, nativePort, embeddedPort]).size !== 3) throw new Error('Driver port allocation collided; retry the test.');
  const windows = process.platform === 'win32';
  // Preserve Microsoft's native diagnostics. On Windows, launch the synthetic
  // app ourselves and attach to its explicit loopback debugging port: the
  // driver's launch mode could not discover this app's DevToolsActivePort file.
  const args = windows ? [`--port=${port}`, '--verbose', ...(nativeLog ? [`--log-path=${nativeLog}`] : [])]
    : ['--port', String(port), '--native-port', String(nativePort)];
  if (!windows && nativeDriver) args.push('--native-driver', nativeDriver);
  const child = spawn(windows ? (nativeDriver ?? 'msedgedriver.exe') : (tauriDriver ?? 'tauri-driver'), args, {
    cwd, stdio: ['ignore', 'pipe', 'pipe'],
    env: { ...process.env, ...env, TAURI_WEBVIEW_AUTOMATION: 'true', TAURI_WEBDRIVER_PORT: String(embeddedPort) },
  });
  let spawnError;
  child.stdout.on('data', (data) => onLog(String(data)));
  child.stderr.on('data', (data) => onLog(String(data)));
  child.on('error', (error) => { spawnError = error; });
  const origin = `http://127.0.0.1:${port}`;
  let session;
  let application;
  let applicationError;

  async function command(method, path, body, timeout = 30_000) {
    if (spawnError) throw spawnError;
    if (child.exitCode !== null) throw new Error(`External driver exited with code ${child.exitCode}.`);
    let response;
    try {
      response = await fetch(`${origin}${path}`, {
        method, headers: body === undefined ? {} : { 'Content-Type': 'application/json' },
        body: body === undefined ? undefined : JSON.stringify(body), signal: AbortSignal.timeout(timeout),
      });
    } catch (error) { throw new Error(`Native WebDriver ${method} ${path}: ${error.message}`, { cause: error }); }
    const result = await response.json();
    if (!response.ok || result.value?.error) {
      throw new Error(`Native WebDriver ${path}: ${result.value?.error ?? response.status}: ${result.value?.message ?? ''}`);
    }
    return result.value;
  }

  async function close() {
    if (session) {
      try { await command('DELETE', `/session/${session}`, undefined, 10_000); } catch { /* keep original failure */ }
      session = undefined;
    }
    child.kill();
    application?.kill();
    // Delete the session before terminating this driver. Never issue a broad
    // browser-process kill, which could affect the user's own sessions.
  }

  try {
    const deadline = Date.now() + 60_000;
    let ready = false;
    while (Date.now() < deadline) {
      if (spawnError) throw spawnError;
      if (child.exitCode !== null) throw new Error('Native driver failed during startup.');
      try { ready = Boolean((await command('GET', '/status', undefined, 2_000)).ready); } catch { /* startup */ }
      if (ready) break;
      await delay(100);
    }
    if (!ready) throw new Error('Native WebDriver startup timed out.');
    if (windows) {
      application = spawn(binary, [], {
        cwd, stdio: ['ignore', 'pipe', 'pipe'],
        env: {
          ...process.env, ...env,
          WEBVIEW2_USER_DATA_FOLDER: resolve(env.MAILX_BODY_WEBVIEW_ROOT, 'webview'),
          WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${nativePort} --remote-debugging-address=127.0.0.1`,
        },
      });
      application.stdout.on('data', (data) => onLog(`[synthetic app stdout] ${data}`));
      application.stderr.on('data', (data) => onLog(`[synthetic app stderr] ${data}`));
      application.on('error', (error) => { applicationError = error; });
      const debugDeadline = Date.now() + 60_000;
      let debuggingReady = false;
      while (Date.now() < debugDeadline) {
        if (applicationError) throw applicationError;
        if (application.exitCode !== null) throw new Error(`Synthetic WebView2 app exited with code ${application.exitCode}.`);
        try {
          const response = await fetch(`http://127.0.0.1:${nativePort}/json/version`, { signal: AbortSignal.timeout(2_000) });
          debuggingReady = response.ok && Boolean((await response.json()).webSocketDebuggerUrl);
        } catch { /* WebView2 initialization */ }
        if (debuggingReady) break;
        await delay(100);
      }
      if (!debuggingReady) throw new Error('Synthetic WebView2 debugging endpoint did not become ready.');
    }
    const created = await command('POST', '/session', {
      capabilities: { alwaysMatch: {
        ...(windows ? { browserName: 'webview2', 'ms:edgeChromium': true, 'ms:edgeOptions': { debuggerAddress: `127.0.0.1:${nativePort}` } }
          : { 'tauri:options': { application: binary } }),
        pageLoadStrategy: 'normal',
      } },
    }, 90_000);
    session = created.sessionId;
    if (!session) throw new Error('Native driver did not return a session id.');
    await command('POST', `/session/${session}/timeouts`, { script: 10_000, pageLoad: 30_000, implicit: 0 });

    const execute = (script, ...args) => command('POST', `/session/${session}/execute/sync`, { script, args });
    const executeAsync = (script, ...args) => command('POST', `/session/${session}/execute/async`, { script, args });
    const top = () => command('POST', `/session/${session}/frame`, { id: null });
    const find = (selector) => command('POST', `/session/${session}/element`, { using: 'css selector', value: selector });
    async function inFrame(operation) {
      await top();
      const frame = await find('#body');
      await command('POST', `/session/${session}/frame`, { id: frame });
      try { return await operation(); } finally { await top(); }
    }
    async function frameSnapshot() {
      return inFrame(async () => {
        const snapshot = await execute(`
          let parentDomReadable=false, parentStateReadable=false, parentInvokeReadable=false;
          const parentAccessErrors={};
          try { parentDomReadable=Boolean(parent.document.body); } catch(e) { parentAccessErrors.dom=e.name; }
          try { parentStateReadable=parent.__MAILX_TEST_SECRET==='synthetic-parent-state'; } catch(e) { parentAccessErrors.state=e.name; }
          try { parentInvokeReadable=typeof parent.__TAURI_INTERNALS__?.invoke==='function'; } catch(e) { parentAccessErrors.invoke=e.name; }
          const link=document.querySelector('a[data-test-link], a');
          const rect=link?.getBoundingClientRect();
          const image=Array.from(document.querySelectorAll('img')).find(image=>
            image.src.startsWith('mailx-body://localhost/part/') || image.src.startsWith('http://mailx-body.localhost/part/'));
          if (!window.__MAILX_EXTERNAL_DOCUMENT_MARKER) window.__MAILX_EXTERNAL_DOCUMENT_MARKER=Date.now()+'-'+Math.random();
          return {
            url:location.href, observerWorld:'page', instrumentation:'native-webdriver-frame-execute',
            documentMarker:window.__MAILX_EXTERNAL_DOCUMENT_MARKER, documentReady:document.readyState,
            title:document.querySelector('h1')?.textContent??null,
            pageScriptExecuted:window.__MAILX_PAGE_SCRIPT_EXECUTED===true || document.documentElement?.dataset.mailxPageScriptExecuted==='true',
            tauriInvokePresent:typeof window.__TAURI_INTERNALS__?.invoke==='function',
            tauriGlobalPresent:typeof window.__TAURI__!=='undefined',
            parentDomReadable,parentStateReadable,parentInvokeReadable,parentAccessErrors,
            inlineImageLoaded:Boolean(image?.complete && image.naturalWidth>0), inlineImageWidth:image?.naturalWidth??0,
            linkRectangle:rect?{x:rect.x,y:rect.y,width:rect.width,height:rect.height}:null, linkHref:link?.href??null
          };`);
        snapshot.ipcProbe = await executeAsync(`
          const done=arguments[arguments.length-1];
          if (typeof window.__TAURI_INTERNALS__?.invoke!=='function') { done('unavailable'); return; }
          Promise.race([
            window.__TAURI_INTERNALS__.invoke('app_get_startup_status',{}).then(()=> 'unexpectedly-resolved',()=> 'denied'),
            new Promise(resolve=>setTimeout(()=>resolve('timed-out'),3000))
          ]).then(done,()=>done('probe-failed'));`);
        return snapshot;
      });
    }
    async function state() {
      await top();
      return execute('return window.__MAILX_BODY_TEST?.state() ?? null;');
    }
    async function observe() {
      const frame = await frameSnapshot();
      const parent = await state();
      return { ...parent, observations: [...(parent.observations ?? []), frame] };
    }
    async function clickLink() {
      return inFrame(async () => {
        const link = await find('a[data-test-link], a');
        // W3C element click is a trusted native-driver interaction. Do not use
        // execute('element.click()') or relax the sandbox to make this pass.
        await command('POST', `/session/${session}/element/${link[ELEMENT]}/click`, {});
      });
    }
    async function clickNumberedLink() {
      await top();
      try {
        const button = await find('#numbered-links button');
        // The numbered fallback is rendered by the trusted parent. Keep this
        // as a W3C element click so the test exercises the native event path,
        // rather than invoking a child or parent DOM .click() from script.
        await command('POST', `/session/${session}/element/${button[ELEMENT]}/click`, {});
      } finally {
        await top();
      }
    }
    return {
      child, capabilities: created.capabilities, origin, session,
      driver: windows ? 'Microsoft Edge WebDriver (WebView2)' : 'official tauri-driver with WebKitWebDriver',
      execute, executeAsync, command, top, inFrame, frameSnapshot, state, observe, clickLink, clickNumberedLink, close,
      screenshot: async () => { await top(); return command('GET', `/session/${session}/screenshot`); },
    };
  } catch (error) {
    await close();
    throw error;
  }
}
