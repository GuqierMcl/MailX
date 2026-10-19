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

export async function startExternalDriver({ binary, cwd, env, tauriDriver, nativeDriver, onLog = () => {} }) {
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
  const args = ['--port', String(port), '--native-port', String(nativePort)];
  if (nativeDriver) args.push('--native-driver', nativeDriver);
  const child = spawn(tauriDriver ?? (process.platform === 'win32' ? 'tauri-driver.exe' : 'tauri-driver'), args, {
    cwd, stdio: ['ignore', 'pipe', 'pipe'],
    env: { ...process.env, ...env, TAURI_WEBVIEW_AUTOMATION: 'true', TAURI_WEBDRIVER_PORT: String(embeddedPort) },
  });
  let spawnError;
  child.stdout.on('data', (data) => onLog(String(data)));
  child.stderr.on('data', (data) => onLog(String(data)));
  child.on('error', (error) => { spawnError = error; });
  const origin = `http://127.0.0.1:${port}`;
  let session;

  async function command(method, path, body, timeout = 30_000) {
    if (spawnError) throw spawnError;
    if (child.exitCode !== null) throw new Error(`External driver exited with code ${child.exitCode}.`);
    const response = await fetch(`${origin}${path}`, {
      method, headers: body === undefined ? {} : { 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body), signal: AbortSignal.timeout(timeout),
    });
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
    const created = await command('POST', '/session', {
      capabilities: { alwaysMatch: { 'tauri:options': { application: binary }, pageLoadStrategy: 'normal' } },
    }, 60_000);
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
    return {
      child, capabilities: created.capabilities, origin, session,
      driver: 'official tauri-driver with native WebKitWebDriver / Microsoft Edge WebDriver',
      execute, executeAsync, command, top, inFrame, frameSnapshot, state, observe, clickLink, close,
      screenshot: async () => { await top(); return command('GET', `/session/${session}/screenshot`); },
    };
  } catch (error) {
    await close();
    throw error;
  }
}
