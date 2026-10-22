import type { Account } from '@/generated/accounts'

/** Preset identification colors; white text stays readable on each in both themes. */
export const ACCOUNT_COLORS = [
  { value: '#2563eb', name: '蓝' },
  { value: '#16a34a', name: '绿' },
  { value: '#d97706', name: '琥珀' },
  { value: '#dc2626', name: '红' },
  { value: '#7c3aed', name: '紫' },
  { value: '#db2777', name: '粉' },
  { value: '#0891b2', name: '青' },
  { value: '#475569', name: '石板灰' },
] as const

/** The first preset no other account uses yet; cycles once all are taken. */
export function nextAccountColor(accounts: Pick<Account, 'color'>[]): string {
  const used = new Set(accounts.map((account) => account.color))
  return (ACCOUNT_COLORS.find((color) => !used.has(color.value)) ?? ACCOUNT_COLORS[accounts.length % ACCOUNT_COLORS.length]).value
}

export function accountInitial(displayName: string): string {
  return Array.from(displayName.trim())[0]?.toUpperCase() ?? '@'
}
