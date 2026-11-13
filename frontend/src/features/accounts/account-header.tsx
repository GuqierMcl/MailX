import type { ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import type { Account, AccountSyncState } from '@/generated/accounts'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import { formatFullTime } from '@/features/mail/format'
import type { MailxClient } from '@/lib/mailx-client'
import { cn } from '@/lib/utils'
import { AccountAvatar } from './account-avatar'
import type { accountIssues } from './account-status'
import { ProviderAuthHelp } from './provider-auth-help'
import { ProviderBadge } from './provider-badge'

/** The profile as it is being edited, so the header follows it before it is saved. */
export type ProfilePreview = { displayName: string; color: string | null; useProviderIcon: boolean }

/** Who the account is and whether it is syncing. The repair actions live in the page's footer. */
export function AccountHeader({ client, account, profile, sync, issues }: {
  client: MailxClient
  account: Account
  profile: ProfilePreview
  sync: AccountSyncState | undefined
  issues: ReturnType<typeof accountIssues>
}) {
  useTranslation()
  const { needsTrust, stopped, oauth, stoppedForSignIn, signInFirst, retrying } = issues
  const stoppedHere = stopped && !stoppedForSignIn ? stopped : null
  const trouble = signInFirst || needsTrust || stoppedHere
  return (
    <header className="flex flex-col gap-3">
      <div className="flex items-center gap-3">
        <AccountAvatar displayName={profile.displayName} color={profile.color} providerId={account.providerId} useProviderIcon={profile.useProviderIcon} className="size-10 text-base" />
        <div className="min-w-0 flex-1">
          <h2 className="truncate text-base font-semibold">{profile.displayName}</h2>
          <p className="truncate text-xs text-muted-foreground">{account.emailAddress}</p>
        </div>
        <ProviderBadge providerId={account.providerId} className="shrink-0" />
      </div>
      {signInFirst && (
        <StatusLine tone="bad">
          <p className="font-medium text-destructive">{t('accounts:signInAgainNeededTitle')}</p>
          <p className="text-muted-foreground">{t('accounts:signInAgainNeededDescription')}</p>
        </StatusLine>
      )}
      {needsTrust && (
        <StatusLine tone="bad">
          <p className="font-medium text-destructive">{t('accounts:credentialsNeedConfirmation')}</p>
          <p className="text-muted-foreground">{oauth ? t('accounts:changedIdentityOAuthHint') : t('accounts:changedIdentityHint')}</p>
        </StatusLine>
      )}
      {!needsTrust && stoppedHere && (
        <StatusLine tone="bad">
          <p className="font-medium text-destructive">{t('accounts:syncStopped')}</p>
          <p className="text-muted-foreground">{errorText(stoppedHere)}  {!oauth && stoppedHere.code !== 'providerPolicy' && t('accounts:retryAfterConnectionChange')}</p>
        </StatusLine>
      )}
      {!trouble && retrying && (
        <StatusLine tone="warn">
          <p className="text-amber-700 dark:text-amber-400">{t('accounts:syncRetrying', { reason: errorText(retrying) })}</p>
        </StatusLine>
      )}
      {!trouble && !retrying && sync?.lastSyncedAt && (
        <p className="text-xs text-muted-foreground">{t('accounts:lastSynced', { time: formatFullTime(sync.lastSyncedAt) })}</p>
      )}
      {!needsTrust && stoppedHere && (stoppedHere.code === 'authentication' || stoppedHere.code === 'providerPolicy') &&
        <ProviderAuthHelp client={client} providerId={account.providerId} reason={stoppedHere.code} />}
    </header>
  )
}

function StatusLine({ tone, children }: { tone: 'bad' | 'warn'; children: ReactNode }) {
  return (
    <div className="flex items-start gap-2 text-xs">
      <span aria-hidden className={cn('mt-1 size-1.5 shrink-0 rounded-full', tone === 'bad' ? 'bg-destructive' : 'bg-amber-500')} />
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">{children}</div>
    </div>
  )
}
