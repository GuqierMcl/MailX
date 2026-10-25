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
