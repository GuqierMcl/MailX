import { errorText } from '@/i18n/errors'
import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import type { Account } from '@/generated/accounts'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Delayed } from '@/components/ui/delayed'
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '@/components/ui/dialog'
import { Skeleton } from '@/components/ui/skeleton'
import { SettingsPage } from '@/features/settings/settings-layout'
import type { MailxClient } from '@/lib/mailx-client'
import { ConnectionForm } from './connection-form'
import { useLeaveGuard } from './leave-guard'
import { ProviderBadge } from './provider-badge'
import { useAccount } from './use-account'

type ConnectionDialogProps = {
  client: MailxClient
  /** The account whose connection is edited; `null` keeps the dialog closed. */
  accountId: string | null
  onClose: () => void
  onSaved: (account: Account) => void
}

/**
 * Edits the rarely changed part of an account: address, servers, credentials
 * and route. Saving restarts the account's sync with the new settings.
 */
export function ConnectionDialog({ client, accountId, onClose, onSaved }: ConnectionDialogProps) {
  useTranslation()
  const guard = useLeaveGuard()
  const { account, error, version, reload } = useAccount(client, accountId)

  return (
    <>
      <Dialog open={accountId !== null} onOpenChange={(open) => !open && guard.leave(onClose)}>
        <DialogContent className="flex h-[min(640px,calc(100dvh-4rem))] flex-col gap-0 overflow-hidden p-0 md:max-w-[640px]">
          <DialogDescription className="sr-only">{t('accounts:editConnectionDescription')}</DialogDescription>
          <header className="flex shrink-0 items-center gap-2 px-6 py-4 pr-12">
            <DialogTitle className="text-base font-semibold">{t('accounts:editConnection')}</DialogTitle>
            {account && <span className="min-w-0 truncate text-sm text-muted-foreground">{account.displayName}</span>}
            {account && <ProviderBadge providerId={account.providerId} className="ml-auto shrink-0" />}
          </header>
          <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
            {error ? (
              <SettingsPage>
                <Alert variant="destructive">
                  <AlertTitle>{t('accounts:loadAccountFailed')}</AlertTitle>
                  <AlertDescription>{errorText(error)}</AlertDescription>
                  <Button variant="outline" className="mt-3 w-fit" onClick={() => void reload()}>{t('accounts:retryLoad')}</Button>
                </Alert>
              </SettingsPage>
            ) : !account ? (
              <SettingsPage>
                <Delayed>
                  <div role="status" aria-label={t('accounts:loadingAccount')} className="flex flex-col gap-3">
                    <Skeleton className="h-16 w-full" />
                    <Skeleton className="h-48 w-full" />
                  </div>
                </Delayed>
              </SettingsPage>
            ) : (
              <ConnectionForm
                key={version}
                account={account}
                onOpenHelp={() => client.openProviderHelp(account.providerId ?? '')}
                intro={{
                  title: t('accounts:reconnectAfterSave'),
                  description: t('accounts:reconnectDescription'),
                }}
                submitLabel={t('accounts:saveAndReconnect')}
                submittingLabel={t('accounts:savingEllipsis')}
                unavailable={!client.available}
                onBusyChange={guard.setBusy}
                onDirtyChange={guard.setDirty}
                onReload={() => {
                  guard.setDirty(false)
                  void reload()
                }}
                onSubmit={async (connection) => {
                  const saved = await client.updateConnection({ accountId: account.id, expectedRevision: account.revision, connection })
                  onSaved(saved)
                }}
              />
            )}
          </div>
        </DialogContent>
      </Dialog>
      {guard.confirm}
    </>
  )
}
