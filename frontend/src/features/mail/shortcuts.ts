/**
 * Keyboard shortcuts of the main window, in one place: the workbench matches
 * key presses against this list, and the help overlay and the settings page
 * render it. Single-key shortcuts can be turned off in the settings; the ones
 * with a modifier (and Esc) are handled where they always were and stay on.
 */

import type { ParseKeys } from 'i18next'

export type ShortcutGroup = 'browse' | 'act' | 'respond'
/** A message key of the `mail` namespace. */
type MailKey = Extract<ParseKeys<['mail']>, `mail:${string}`>

/** Single-key shortcuts, handled by `useMailShortcuts`. */
export type SingleKeyShortcutId =
  | 'next' | 'previous' | 'open' | 'search' | 'help'
  | 'archive' | 'delete' | 'star' | 'read'
  | 'reply' | 'replyAll' | 'forward'

/** Shortcuts with a modifier, or Esc: listed only, each is handled by its own component. */
type FixedShortcutId = 'searchModifier' | 'close' | 'sidebar' | 'selectAll' | 'compose'

export type ShortcutDefinition = {
  id: SingleKeyShortcutId | FixedShortcutId
  /** Alternatives, each a sequence of key caps, e.g. `[['J'], ['↓']]` or `[['⌘', 'N']]`. */
  keys: string[][]
  group: ShortcutGroup
  labelKey: MailKey
  /** Turned off with the "single-key shortcuts" setting. */
  singleKey: boolean
  /** `KeyboardEvent.key` values that run it (letters lower case); single-key shortcuts only. */
  match?: string[]
}

export const IS_MAC = /Mac|iPhone|iPad/.test(globalThis.navigator?.platform ?? '')
/** The platform's command modifier as shown on a key cap. */
export const MOD_KEY = IS_MAC ? '⌘' : 'Ctrl'

export const SHORTCUTS: ShortcutDefinition[] = [
  { id: 'next', keys: [['J'], ['↓']], group: 'browse', labelKey: 'mail:shortcutNext', singleKey: true, match: ['j', 'ArrowDown'] },
  { id: 'previous', keys: [['K'], ['↑']], group: 'browse', labelKey: 'mail:shortcutPrevious', singleKey: true, match: ['k', 'ArrowUp'] },
  { id: 'open', keys: [['Enter'], ['O']], group: 'browse', labelKey: 'mail:shortcutOpen', singleKey: true, match: ['Enter', 'o'] },
  { id: 'close', keys: [['Esc']], group: 'browse', labelKey: 'mail:shortcutClose', singleKey: false },
  { id: 'search', keys: [['/']], group: 'browse', labelKey: 'mail:shortcutSearch', singleKey: true, match: ['/'] },
  { id: 'searchModifier', keys: [[MOD_KEY, 'K']], group: 'browse', labelKey: 'mail:shortcutPalette', singleKey: false },
  { id: 'sidebar', keys: [[MOD_KEY, 'B']], group: 'browse', labelKey: 'mail:shortcutSidebar', singleKey: false },
  { id: 'help', keys: [['?']], group: 'browse', labelKey: 'mail:shortcutHelp', singleKey: true, match: ['?'] },
  { id: 'archive', keys: [['E']], group: 'act', labelKey: 'mail:shortcutArchive', singleKey: true, match: ['e'] },
  {
    id: 'delete',
    keys: IS_MAC ? [['#'], ['⌫'], ['⌦']] : [['#'], ['Delete'], ['Backspace']],
    group: 'act', labelKey: 'mail:shortcutDelete', singleKey: true, match: ['#', 'Delete', 'Backspace'],
  },
  { id: 'star', keys: [['S']], group: 'act', labelKey: 'mail:shortcutStar', singleKey: true, match: ['s'] },
  { id: 'read', keys: [['U']], group: 'act', labelKey: 'mail:shortcutRead', singleKey: true, match: ['u'] },
  { id: 'selectAll', keys: [[MOD_KEY, 'A']], group: 'act', labelKey: 'mail:shortcutSelectAll', singleKey: false },
  { id: 'reply', keys: [['R']], group: 'respond', labelKey: 'mail:reply', singleKey: true, match: ['r'] },
  { id: 'replyAll', keys: [['A']], group: 'respond', labelKey: 'mail:replyAll', singleKey: true, match: ['a'] },
  { id: 'forward', keys: [['F']], group: 'respond', labelKey: 'mail:forward', singleKey: true, match: ['f'] },
  { id: 'compose', keys: [[MOD_KEY, 'N']], group: 'respond', labelKey: 'mail:shortcutCompose', singleKey: false },
]

export const SHORTCUT_GROUPS: { id: ShortcutGroup; labelKey: MailKey }[] = [
  { id: 'browse', labelKey: 'mail:shortcutsGroupBrowse' },
  { id: 'act', labelKey: 'mail:shortcutsGroupAct' },
  { id: 'respond', labelKey: 'mail:shortcutsGroupRespond' },
]

/** Places that take keys for themselves: text entry of any kind. */
const TEXT_ENTRY = [
  'input', 'textarea', 'select',
  '[contenteditable]:not([contenteditable="false"])',
  '[role="textbox"]', '[role="combobox"]', '[role="searchbox"]', '[role="spinbutton"]',
].join(', ')

/** An open dialog, menu or popup owns the keyboard while it is up. */
const OPEN_LAYER = '[role="dialog"][data-open], [role="alertdialog"][data-open], [role="dialog"][open], [role="menu"], [role="listbox"]'

/** Controls that act on Enter themselves (a focused row's button opens it already). */
const ACTS_ON_ENTER = 'button, a[href], summary, [role="button"], [role="link"], [role="tab"], [role="checkbox"], [role="switch"], [role="radio"], [role="option"], [role="menuitem"]'

/** Controls that move with the arrow keys themselves. */
const USES_ARROWS = '[role="tablist"], [role="radiogroup"], [role="slider"], [role="separator"], [role="tree"], [role="grid"]'

/**
 * Whether a single-key shortcut must leave this key press alone: already
 * handled, typing (including IME composition), a modifier other than Shift,
 * focus in a text field, or a dialog, menu or popup open.
 */
export function singleKeyBlocked(event: KeyboardEvent) {
  if (event.defaultPrevented || event.isComposing || event.keyCode === 229) return true
  if (event.metaKey || event.ctrlKey || event.altKey) return true
  const target = event.target instanceof Element ? event.target : null
  if (target && (target.closest(TEXT_ENTRY) || (target as HTMLElement).isContentEditable)) return true
  return Boolean(document.querySelector(OPEN_LAYER))
}

/**
 * The single-key shortcut a key press asks for, or null. Uses `event.key`, so
 * the user's keyboard layout decides. Shift is accepted only for symbols that
 * may need it on some layouts (`#`, `?`, `/`); Shift with a letter, Enter or an
 * arrow is someone else's.
 */
export function matchShortcut(event: KeyboardEvent): SingleKeyShortcutId | null {
  if (singleKeyBlocked(event)) return null
  const key = event.key.length === 1 ? event.key.toLowerCase() : event.key
  const letter = /^[a-z]$/.test(key)
  if (event.shiftKey && (letter || key.length > 1)) return null
  const shortcut = SHORTCUTS.find((item) => item.match?.includes(key))
  if (!shortcut) return null
  const target = event.target instanceof Element ? event.target : null
  if (key === 'Enter' && target?.closest(ACTS_ON_ENTER)) return null
  if ((key === 'ArrowDown' || key === 'ArrowUp') && target?.closest(USES_ARROWS)) return null
  return shortcut.id as SingleKeyShortcutId
}
