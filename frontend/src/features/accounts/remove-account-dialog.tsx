import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { Account, AccountRemoval } from '@/generated/accounts'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import { clientFailure, type ClientFailure, type MailxClient } from '@/lib/mailx-client'
import { Button } from '@/components/ui/button'
import { toast } from '@/components/ui/toast'
import {
  AlertDialog, AlertDialogCancel, AlertDialogContent, AlertDialogDescription,
  AlertDialogFooter, AlertDialogHeader, AlertDialogTitle,
} from '@/components/ui/alert-dialog'

type Summary = { drafts: number; queued: number; unknown: number }

export function RemoveAccountDialog({ account, client, onClose, onBusyChange, onRemoved, onReload }: {
  account: Account
  client: MailxClient
  onClose: () => void
  onBusyChange: (busy: boolean) => void
  onRemoved: (result: AccountRemoval) => void
  onReload: () => void
}) {
  useTranslation()
  const [summary, setSummary] = useState<Summary | null>(null)
  const [failure, setFailure] = useState<ClientFailure | null>(null)
  const [busy, setBusy] = useState(false)
  const [version, setVersion] = useState(0)
  const pending = useRef(false)
  const cancel = useRef<HTMLButtonElement | null>(null)
  useEffect(() => {
    let alive = true
    void Promise.all([client.listDrafts(account.id), client.listOutbox({ accountId: account.id })]).then(([drafts, outbox]) => {
      if (alive) setSummary({ drafts: drafts.length, queued: outbox.filter((item) => item.state === 'queued').length, unknown: outbox.filter((item) => item.state === 'unknown' && !item.resolution).length })
    }, (cause) => { if (alive) setFailure(clientFailure(cause)) })
    return () => { alive = false }
  }, [client, account.id, version])

  async function remove() {
    if (pending.current || !summary) return
    pending.current = true
    setBusy(true)
    setFailure(null)
    onBusyChange(true)
    try {
      const result = await client.removeAccount({ accountId: account.id, expectedRevision: account.revision })
      onRemoved(result)
      toast.success(t('accounts:accountRemoved'), result.credentialsPending ? { description: t('accounts:credentialCleanupPending') } : undefined)
    } catch (cause) {
      setFailure(clientFailure(cause))
    } finally {
      pending.current = false
      setBusy(false)
      onBusyChange(false)
    }
  }

  return (
    <AlertDialog open onOpenChange={(open) => { if (!open && !pending.current) onClose() }}>
      <AlertDialogContent className="sm:max-w-md" initialFocus={cancel}>
        <AlertDialogHeader>
          <AlertDialogTitle>{t('accounts:removeAccountTitle', { name: account.displayName })}</AlertDialogTitle>
          <AlertDialogDescription className="space-y-3">
            <span className="block">{account.emailAddress}</span>
            <span className="block text-sm text-foreground">{t('accounts:removeAccountDescription')}</span>
            <span className="block">{t('accounts:removeAccountComposerHint')}</span>
            <span className="block">{t('accounts:removeAccountDeliveryHint')}</span>
          </AlertDialogDescription>
        </AlertDialogHeader>
        <div className="space-y-3 text-sm">
          {summary ? <p className="text-xs text-muted-foreground">{t('accounts:removeAccountSummary', { drafts: summary.drafts, queued: summary.queued })}</p> : !failure && <p role="status">{t('accounts:loadingRemovalSummary')}</p>}
          {!!summary?.unknown && <p className="text-destructive">{t('accounts:removeAccountUnknown', { unknown: summary.unknown })}</p>}
          {failure && <div role="alert" className="space-y-2 text-destructive">
            <p>{failure.code === 'alreadySending' ? t('accounts:removeAccountSending') : failure.code === 'conflict' ? t('accounts:removeAccountConflict') : errorText(failure)}</p>
            {!summary && <Button variant="outline" onClick={() => { setFailure(null); setVersion((value) => value + 1) }}>{t('accounts:retryLoad')}</Button>}
            {failure.code === 'conflict' && <Button variant="outline" onClick={onReload}>{t('accounts:retryLoad')}</Button>}
          </div>}
        </div>
        <AlertDialogFooter>
          <AlertDialogCancel ref={cancel} disabled={busy}>{t('composer:action.cancel')}</AlertDialogCancel>
          <Button variant="destructive" disabled={!summary || busy || !client.available} onClick={() => void remove()}>
            {t(busy ? 'accounts:removingAccount' : 'accounts:removeAccount')}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  )
}
