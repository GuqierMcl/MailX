import type { LayoutSettings } from '@/generated/accounts'
import { NAV_DEFAULT, NAV_MIN } from './nav-width'

/** Keep in sync with `LIST_WIDTH_MIN` / `LIST_WIDTH_MAX` in core/src/settings.rs. */
export const LIST_MIN = 300
export const LIST_MAX = 2000
export const LIST_DEFAULT = 420
/** The reader never gets narrower than this; the window gives way before it does. */
export const READER_MIN = 360

/**
 * Below this window width the narrowest full sidebar no longer fits beside the
 * list and reader (plus the 0.5rem gutters), so the sidebar shows as its rail.
 */
export const NAV_RAIL_WINDOW = NAV_MIN + LIST_MIN + READER_MIN + 16

export const DEFAULT_LAYOUT: LayoutSettings = { navWidth: NAV_DEFAULT, navCollapsed: false, listWidth: LIST_DEFAULT, readerCollapsed: false, accountOrder: [] }

export function clampListWidth(width: number) {
  return Math.min(LIST_MAX, Math.max(LIST_MIN, Math.round(width)))
}
