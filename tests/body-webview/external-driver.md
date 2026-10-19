# Native WebDriver route for the synthetic body harness

This route exercises Linux and Windows. Evidence from each actual run is
required; the presence of these files does not establish a passing gate.
The initial Linux run loaded the isolated document and CID image, but its 204
link response navigated to a blank document. Linux therefore uses the numbered
fallback described in [README.md](README.md). macOS has no official
`tauri-driver` backend; keep its separate native verification path.

The embedded driver's frame helper evaluates through `parent.contentWindow`,
which cannot observe an opaque sandboxed frame. The native adapter instead uses
W3C `Find Element` → `Switch To Frame` → `Execute Script`. Observation scripts
run through browser automation, with the empty sandbox and production CSP kept
intact. Successful automation is evidence about the observed page state, not
permission for scripts in email to execute.

## Linux CI

Install the existing Tauri build packages plus `webkit2gtk-driver`, `xvfb`,
`xauth`, and `dbus-x11`. `WebKitWebDriver` must correspond to the installed
WebKitGTK libraries. On Ubuntu 24.04 the application uses WebKitGTK 4.1.

```sh
cargo install tauri-driver --locked --version 2.1.0
bun run body-webview:build
dbus-run-session -- xvfb-run -a bun tests/body-webview/run-external.mjs
```

`TAURI_WEBVIEW_AUTOMATION=true` is passed to the native driver and application.
Wry then enables WebKit automation for its context. Do not disable the WebKit
sandbox, turn on `allow-scripts`, or inject a fake successful observation.
The runner writes assertions, native-driver capabilities, binary/source hashes,
screenshots, and driver logs to `target/body-webview-evidence/linux-external`.
Windows uses the corresponding `win32-external` directory.

## Windows CI

Use the existing `windows-2025` runner's interactive desktop and WebView2 Runtime.
The adapter starts Microsoft Edge WebDriver directly, preserving verbose
native-driver logs even when session creation fails. Its launch mode failed to
discover `DevToolsActivePort` before any assertions ran. The adapter now uses
Microsoft's documented attach approach: it launches only the synthetic test app,
captures that process's output, and attaches through a dynamically allocated
loopback debugging port. The WebView2 profile stays under the synthetic test
root. These process-local test options do not alter the production application,
sandbox or CSP. The embedded macOS driver is not initialized.
Install a driver matching that Runtime, rather than matching Microsoft Edge:

```powershell
pwsh -File tests/body-webview/install-webview2-driver.ps1
bun run body-webview:build
bun tests/body-webview/run-external.mjs --native-driver="$((Resolve-Path target/body-webview-driver/msedgedriver.exe).Path)"
```

The runner accepts `--binary=`, `--output=`, `--tauri-driver=`, and
`--native-driver=` overrides. Preserve `versions.json` with the test evidence. Missing Runtime,
unavailable matching driver, or session-version mismatch is an unverified gate,
not a passing result. The installer downloads only from Microsoft's driver host
and extracts only its executable; it does not install/update WebView2 or change
machine registry settings.

## Adapter integration

The existing test runner creates the loopback canary, random nonce, and temporary
root containing `.mailx-synthetic-webview-test`. Pass those through `env`; the
adapter owns application startup (directly on Windows, through the driver on
Linux), so do not separately spawn the application for this route.

```js
import { startExternalDriver } from '../tests/body-webview/external-driver.mjs';
const driver = await startExternalDriver({
  binary, cwd: root, nativeDriver, onLog: appendLog,
  env: {
    MAILX_BODY_WEBVIEW_ROOT: testRoot,
    MAILX_BODY_WEBVIEW_NONCE: nonce,
    MAILX_BODY_CANARY_ORIGIN: canaryOrigin,
  },
});
try {
  await driver.execute('return window.__MAILX_BODY_TEST.load(arguments[0]);', 'safe');
  // Poll observe() until the selected document/CID is ready. Each call returns
  // a parent state plus a fresh directly observed frame snapshot.
  const snapshot = await driver.observe();
  await driver.clickLink(); // W3C native element click, not DOM element.click()
  const after = await driver.observe();
  // Run the same isolation, 204/documentMarker, real event and canary assertions.
} finally {
  await driver.close();
}
```

`driver.state()` reads the trusted parent only; `frameSnapshot()` directly reads
the selected frame and attempts the harmless startup-status IPC probe only if
an invoke function exists. `observe()` combines the two without adding any
`postMessage` privilege. `screenshot()` returns a base64 PNG. Capture returned
capabilities (browser/runtime versions), assertion outcomes, screenshots, logs,
and canary requests as usual. If native frame selection or execution fails, keep
that failure; do not revert to parent-origin frame eval or relax isolation.

Sources checked when preparing this route:

- [Tauri WebDriver](https://v2.tauri.app/develop/tests/webdriver/)
- [Tauri WebDriver CI](https://v2.tauri.app/develop/tests/webdriver/ci/)
- [tauri-driver capability mapping](https://github.com/tauri-apps/tauri/blob/dev/crates/tauri-driver/src/server.rs)
- [tauri-driver native launch](https://github.com/tauri-apps/tauri/blob/dev/crates/tauri-driver/src/webdriver.rs)
- [Microsoft WebView2 WebDriver](https://learn.microsoft.com/en-us/microsoft-edge/webview2/how-to/webdriver)
- [WebKit Automation frame JavaScript evaluation](https://github.com/WebKit/WebKit/blob/main/Source/WebKit/WebProcess/Automation/WebAutomationSessionProxy.cpp)
