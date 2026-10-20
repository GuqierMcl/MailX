# Real WebView checks

The test feature exercises the production renderer, cache, `mailx-body` protocol,
headers, CID validation and Tauri command/event path in a real platform WebView.
It starts with a new synthetic profile and must never initialize user accounts,
keychain access or synchronization. The initial three-platform gate was completed
before the product reading UI was implemented; see the validation record.

During MVP development these checks run on demand through the `Body WebView`
workflow (`.github/workflows/body-webview.yml`, `workflow_dispatch`), rather than
on every pull request or main-branch push. Once the workflow exists on the default
branch, select the desired branch in GitHub Actions and run it manually. The
three-platform matrix and evidence artifacts are retained. Run relevant checks
after rendering, CSP, resource-protocol, link-navigation or Tauri / Wry changes,
and exercise all supported platforms before release. Ordinary frontend and Rust
unit tests remain part of automatic CI.

## Drivers

| Platform | Driver and opaque-frame observation | Link input |
| --- | --- | --- |
| macOS | Embedded `tauri-plugin-wdio-webdriver` 1.5.0 for the main window; public WKWebView frame evaluation, Vision OCR and screenshot pixels for the body | AppKit mouse events on the inline link |
| Windows | Matching Microsoft `msedgedriver` with WebView2 capabilities, native WebDriver frame selection | W3C native element click in the body |
| Linux | Official `tauri-driver` 2.1.0 + WebKitWebDriver, native WebDriver frame selection | W3C native element click in the body |

The embedded driver's frame implementation uses parent `contentWindow.eval`,
which cannot access an opaque sandboxed child. Its synthetic DOM click path is
therefore not used for body-frame evidence. See [external-driver.md](external-driver.md)
for the Windows/Linux adapter and verified upstream APIs.

Upstream references checked on 2026-10-19:
[Tauri WebDriver guide](https://v2.tauri.app/develop/tests/webdriver/),
[embedded plugin](https://crates.io/crates/tauri-plugin-wdio-webdriver/1.5.0),
[embedded plugin source](https://github.com/webdriverio/desktop-mobile/tree/main/packages/tauri-plugin-webdriver).

## Compile-time isolation

`body-webview-test` is a non-default Cargo feature with optional test dependencies.
It selects a separate startup path, creates synthetic MIME through the real cache,
registers the test driver, and embeds only the assets in this directory. The build
subcommand supplies a temporary Tauri configuration with these assets and the
**unchanged production CSP**. Normal builds contain no test driver, test startup,
fixture replacement, test assets or native helper.

Safe and malicious fixtures pass through the production renderer. `policyProbe`
replaces only its response body, after normal identity/token validation, with
active synthetic HTML under the same production headers and resource scope. Its
unique title must be observed, so an ineffective override cannot pass the test.
The attack tries to replace that title, mutate parent state, invoke Tauri, send a
forged message and contact a loopback canary. An image targeting the link route
must not even reach the handler; a 404 cannot mask a failed image CSP restriction.

## macOS observation and inline navigation

On the locally tested macOS 26.2 WebView, page-world user scripts and explicit
`evaluateJavaScript:inFrame:inContentWorld:` calls are rejected inside the empty
sandbox. Named isolated worlds are also rejected. Missing telemetry is not a
pass. The runner requires explicit `WKErrorDomain` code 4 with `Cannot execute
JavaScript in this document` for page-script, global access, parent DOM/state,
parent invocation and local invocation probes. The report marks the globals as
**unobservable**, not absent. A different native evaluation result fails until
that platform's actual behavior has been investigated.

`native/observer.m` uses public AppKit/WebKit APIs. The runner compiles a temporary
dylib and sets `DYLD_INSERT_LIBRARIES` only on the synthetic child process. The
helper forwards the original navigation delegate's policy unchanged, obtains
`WKFrameInfo`, and records explicit evaluation outcomes in the trusted main page.
No script-message handler or privileged object is installed in body frames.
Rust's `unsafe_code = forbid` remains intact.

`native/inspect.swift` uses Vision OCR to verify each fixture's exact title and
reads screenshot samples to identify its 96 × 96 red CID image. The expected
text and red image must remain visible after clicking a link. The parent also
records actual `SecurityError` exceptions when accessing the body frame.

The original implementation used parent numbered links because WKWebView and
WebKitGTK replaced their documents after a custom-protocol 204 response. That
historical evidence remains recorded in the validation document. Current builds
cancel link navigation in Tauri's native callback before commitment, then emit
the same bound event to the trusted parent. WebView2 retains a protocol 204
fallback. The harness clicks the inline link, checks the database URL, unchanged
document/frame load count, preserved text and CID image, and zero external
requests. It never calls `links_open` or opens a browser.

Before loading any document, the trusted parent obtains a host-issued token
through `emails_prepare_body`. The document URL and subsequent link metadata
use this exact token; a document GET never creates a token or changes another
view's binding. Events carry both the token and a unique request ID. The parent
rejects events from a previous view, including a previous view of the same
email. The trusted opener consumes that exact request once; the product UI no longer asks for a per-click confirmation.

## Run

```sh
bun scripts/body-webview.mjs build
# macOS: native AppKit mouse events in the synthetic process, no Accessibility grant needed
bun scripts/body-webview.mjs run --input=instrumented
# Windows: use matching msedgedriver; see external-driver.md
bun scripts/body-webview.mjs run --native-driver=C:/path/to/msedgedriver.exe
# Linux: install WebKitWebDriver and platform dependencies, then run with D-Bus/Xvfb
 dbus-run-session -- xvfb-run -a bun scripts/body-webview.mjs run
```

On macOS, `--input=native` additionally supports the CoreGraphics helper in
`native/click.swift` and requires Accessibility permission. `--input=manual`
writes `click-ready.json` and waits up to 60 seconds for CUA or a physical click.
`--input=instrumented` uses public `NSWindow.sendEvent` mouseDown/mouseUp,
not DOM `.click()`, and labels it as native event injection, not physical input.
Windows/Linux always use the external runner regardless of the macOS input flag.

Each report records Git SHA, dirty state/diff digest, binary SHA-256, platform,
user agent, capabilities, input method, assertions, protocol statuses, remote
canary requests and screenshots. The macOS report also hashes its native/helper
sources. Failure returns nonzero and preserves evidence. Neither sandbox tokens
nor CSP are relaxed, and a missing platform/driver/observation never counts as
passing. CI uploads evidence even on failure and publishes a synthetic-only
summary through GitHub check annotations for easier failure diagnosis.
