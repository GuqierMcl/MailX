import type { LayoutSettings } from '@/generated/accounts'
import { NAV_DEFAULT } from './nav-width'

/** Keep in sync with `LIST_WIDTH_MIN` / `LIST_WIDTH_MAX` in core/src/settings.rs. */
export const LIST_MIN = 300
export const LIST_MAX = 2000
export const LIST_DEFAULT = 420
/** The reader never gets narrower than this; the window gives way before it does. */
export const READER_MIN = 360

export const DEFAULT_LAYOUT: LayoutSettings = { navWidth: NAV_DEFAULT, listWidth: LIST_DEFAULT, readerCollapsed: false }

export function clampListWidth(width: number) {
  return Math.min(LIST_MAX, Math.max(LIST_MIN, Math.round(width)))
}
