/**
 * Keeps the desktop window from behaving like a browser: no page context menu,
 * no reload, print, find, view-source, developer-tools or history shortcuts.
 * Only preventDefault is used, never stopPropagation, so the app can still bind
 * the same keys to its own actions. Editing keys (copy, paste, select all,
 * undo) and Tab navigation are untouched.
 *
 * This covers the app document only. The mail body lives in a script-less
 * sandboxed frame that page scripts cannot reach; the desktop host handles
 * that frame natively where the platform allows it.
 */

const EDITABLE =
  'input:not([type="checkbox"],[type="radio"],[type="button"],[type="submit"],[type="reset"],' +
  '[type="range"],[type="color"],[type="file"],[type="image"]),textarea,[contenteditable=""],[contenteditable="true"]'

/** Text fields keep their native edit menu (cut, copy, paste, spelling). */
export function isEditable(target: EventTarget | null) {
  return target instanceof Element && target.closest(EDITABLE) !== null
}

type KeyDescription = Pick<KeyboardEvent, 'code' | 'ctrlKey' | 'metaKey' | 'altKey' | 'shiftKey'>

const ALWAYS = new Set(['F3', 'F5', 'F7', 'F12'])
/** Reload, print, save, view source, open, find, find next, downloads. */
const WITH_MODIFIER = new Set(['KeyR', 'KeyP', 'KeyS', 'KeyU', 'KeyO', 'KeyF', 'KeyG', 'KeyJ'])
const DEVTOOLS = new Set(['KeyI', 'KeyJ', 'KeyC'])

/** `code` is the physical key, so the rules hold on non-Latin layouts too. */
export function isBrowserShortcut(event: KeyDescription) {
  const { code, ctrlKey, metaKey, altKey, shiftKey } = event
  const modifier = ctrlKey || metaKey
  if (ALWAYS.has(code)) return true
  if (modifier && !altKey && WITH_MODIFIER.has(code)) return true
  if (modifier && shiftKey && DEVTOOLS.has(code)) return true
  if (metaKey && altKey && (DEVTOOLS.has(code) || code === 'KeyU')) return true
  // Back and forward: Alt+Arrow on Windows and Linux, Cmd+bracket on macOS.
  if (altKey && !modifier && (code === 'ArrowLeft' || code === 'ArrowRight')) return true
  return metaKey && !altKey && (code === 'BracketLeft' || code === 'BracketRight')
}

/** Returns a function that removes the guards again. */
export function installBrowserDefaultGuards(target: Window = window) {
  const onContextMenu = (event: MouseEvent) => {
    if (!isEditable(event.target)) event.preventDefault()
  }
  const onKeyDown = (event: KeyboardEvent) => {
    if (isBrowserShortcut(event)) event.preventDefault()
  }
  target.addEventListener('contextmenu', onContextMenu, true)
  target.addEventListener('keydown', onKeyDown, true)
  return () => {
    target.removeEventListener('contextmenu', onContextMenu, true)
    target.removeEventListener('keydown', onKeyDown, true)
  }
}
