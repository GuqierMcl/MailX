import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { MailsIcon } from '@hugeicons/core-free-icons'
import type { Account, SelectedEmail } from '@/generated/accounts'
import { t } from '@/i18n'

/**
 * What the reader shows while messages are being picked: how many, and from
 * which accounts. A calm placeholder only; the actions live in the list header.
 */
export function SelectionSummary({ items, accounts }: { items: SelectedEmail[]; accounts: Account[] }) {
  useTranslation()
  const perAccount = accounts
    .map((account) => ({ account, count: items.filter((item) => item.accountId === account.id).length }))
    .filter((row) => row.count > 0)
  return (
    <div className="flex h-full flex-col items-center justify-center gap-3 p-6 text-center">
      <HugeiconsIcon icon={MailsIcon} className="size-10 text-muted-foreground" aria-hidden />
      <p className="text-base font-medium">{t('mail:selectionSummary', { count: items.length })}</p>
      {perAccount.length > 0 && (
        <p className="text-sm text-muted-foreground">
          {perAccount.map(({ account, count }) => `${account.displayName} ${count}`).join(' · ')}
        </p>
      )}
    </div>
  )
}
