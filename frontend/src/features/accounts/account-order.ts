import { useMemo } from 'react'
import { useSettings } from '@/features/settings/settings-queries'
import type { MailxClient } from '@/lib/mailx-client'

const NO_ORDER: readonly string[] = []

/** Display preferences never change the account list used for sender resolution. */
export function sortAccountsForDisplay<T extends { id: string }>(accounts: readonly T[], order: readonly string[]): T[] {
  const positions = new Map<string, number>()
  for (const id of order) if (!positions.has(id)) positions.set(id, positions.size)
  // Stable sorting preserves the original order of accounts absent from the preference.
  return [...accounts].sort((left, right) =>
    (positions.get(left.id) ?? positions.size) - (positions.get(right.id) ?? positions.size))
}

/** Settings events keep every window's account menus in the same display order. */
export function useOrderedAccounts<T extends { id: string }>(client: MailxClient, accounts: readonly T[]): T[] {
  const settings = useSettings(client)
  const order = settings.data?.settings.layout.accountOrder ?? NO_ORDER
  return useMemo(() => sortAccountsForDisplay(accounts, order), [accounts, order])
}
