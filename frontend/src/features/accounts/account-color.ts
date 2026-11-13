import { t } from '@/i18n'
import type { Account } from '@/generated/accounts'

/** Preset identification colors; white text stays readable on each in both themes. */
export const ACCOUNT_COLORS = [
  { value: '#2563eb', get name() { return t('accounts:blue') } },
  { value: '#16a34a', get name() { return t('accounts:green') } },
  { value: '#d97706', get name() { return t('accounts:amber') } },
  { value: '#dc2626', get name() { return t('accounts:red') } },
  { value: '#7c3aed', get name() { return t('accounts:purple') } },
  { value: '#db2777', get name() { return t('accounts:pink') } },
  { value: '#0891b2', get name() { return t('accounts:cyan') } },
  { value: '#475569', get name() { return t('accounts:slate') } },
] as const

/** The first preset no other account uses yet; cycles once all are taken. */
export function nextAccountColor(accounts: Pick<Account, 'color'>[]): string {
  const used = new Set(accounts.map((account) => account.color))
  return (ACCOUNT_COLORS.find((color) => !used.has(color.value)) ?? ACCOUNT_COLORS[accounts.length % ACCOUNT_COLORS.length]).value
}

export function accountInitial(displayName: string): string {
  return Array.from(displayName.trim())[0]?.toUpperCase() ?? '@'
}

/** WCAG relative luminance of a `#rrggbb` color, or null for anything else. */
function luminance(color: string): number | null {
  const match = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(color)
  if (!match) return null
  const [r, g, b] = [match[1], match[2], match[3]].map((hex) => {
    const channel = parseInt(hex, 16) / 255
    return channel <= 0.03928 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4
  })
  return 0.2126 * r + 0.7152 * g + 0.0722 * b
}

/**
 * Text color for a mark on an identification color. White unless its contrast
 * falls below 3:1 (luminance above 0.3), so every preset keeps white text and
 * only light custom colors switch to black.
 */
export function accountTextColor(color: string): '#ffffff' | '#000000' {
  const value = luminance(color)
  return value !== null && value > 0.3 ? '#000000' : '#ffffff'
}
