# Real WebView security gate

The test feature exercises the production renderer, cache, `mailx-body` protocol,
headers, CID validation and Tauri command/event path in a real platform WebView.
It starts with a new synthetic profile and must never initialize user accounts,
keychain access or synchronization. The product reading UI remains gated until
all three platforms have evidence.

## Drivers

| Platform | Driver and opaque-frame observation | Link input |
| --- | --- | --- |
| macOS | Embedded `tauri-plugin-wdio-webdriver` 1.5.0 for the main window; public WKWebView frame evaluation, Vision OCR and screenshot pixels for the body | AppKit mouse events on a trusted numbered-link button |
| Windows | Official `tauri-driver` 2.1.0 + matching `msedgedriver`, native WebDriver frame selection | W3C native element click in the body |
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

## macOS observation and link fallback

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

Real AppKit click evidence found that WKWebView delivers the original `/link`
request and 204 response but replaces the document with a blank frame. That
failed evidence is retained separately; it is never relabeled as passing. The
macOS production fallback removes anchor hrefs, adds passive `[N]` labels, and
lets the trusted parent display database URLs. After the document loads, the
test parent calls `emails_body_links` and a real button calls `links_request`
with the current document token. The test checks the real link event and URL,
no extra frame load/document request, preserved rendered content, and zero
`/link` protocol requests. It never calls `links_open` or opens a browser.

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
`--input=instrumented` uses public `NSApplication.sendEvent` mouseDown/mouseUp,
not DOM `.click()`, and labels it as native event injection, not physical input.
Windows/Linux always use the external runner regardless of the macOS input flag.

Each report records Git SHA, dirty state/diff digest, binary SHA-256, platform,
user agent, capabilities, input method, assertions, protocol statuses, remote
canary requests and screenshots. The macOS report also hashes its native/helper
sources. Failure returns nonzero and preserves evidence. Neither sandbox tokens
nor CSP are relaxed, and a missing platform/driver/observation never counts as
passing. CI uploads evidence even on failure.
