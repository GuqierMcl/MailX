/** Keep in sync with `NAV_WIDTH_MIN` / `NAV_WIDTH_MAX` in core/src/settings.rs. */
export const NAV_MIN = 220
export const NAV_MAX = 420
export const NAV_DEFAULT = 256

export function clampNavWidth(width: number) {
  return Math.min(NAV_MAX, Math.max(NAV_MIN, Math.round(width)))
}
