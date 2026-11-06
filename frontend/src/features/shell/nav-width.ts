/** Keep in sync with `NAV_WIDTH_MIN` / `NAV_WIDTH_MAX` in core/src/settings.rs. */
export const NAV_MIN = 220
export const NAV_MAX = 420
export const NAV_DEFAULT = 256

/** Width of the folded sidebar: one column of icons. */
export const NAV_RAIL = 48

/**
 * A drag that leaves the sidebar narrower than this folds it to the rail, and
 * one that takes the rail wider opens it again: halfway between the two.
 */
export const NAV_SNAP = Math.round((NAV_RAIL + NAV_MIN) / 2)

export function clampNavWidth(width: number) {
  return Math.min(NAV_MAX, Math.max(NAV_MIN, Math.round(width)))
}
