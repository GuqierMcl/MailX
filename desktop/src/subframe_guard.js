// Appended to Tauri's IPC setup, which runs before the scripts that install
// `invoke`, callbacks and the global API. WebView2 runs every initialization
// script in every frame (wry ignores main-frame-only there), so an untrusted
// subframe such as an email body would otherwise get the IPC bridge. Freezing
// the internals makes those later scripts fail in subframes only.
;(function () {
  if (window.top === window) return
  for (const name of ['__TAURI__', '__TAURI_IIFE__']) {
    Object.defineProperty(window, name, { value: undefined })
  }
  const internals = window.__TAURI_INTERNALS__
  if (internals) {
    Object.freeze(internals.plugins)
    Object.freeze(internals)
  }
})()
