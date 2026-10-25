import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import { useQuery } from '@tanstack/react-query'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Delayed } from '@/components/ui/delayed'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Spinner } from '@/components/ui/spinner'
import { loadMailTree } from '@/features/mail/mail-tree'
import type { MailxClient } from '@/lib/mailx-client'
import { mailKeys } from '@/lib/query-client'
import { SettingsGroup, SettingsPage, SettingsRow } from './settings-layout'
import { useSettings, useUpdateCompose } from './settings-queries'

/** Choices the core accepts for `compose.undoSendSeconds`. */


/** Empty value: no fixed account, so a new mail follows the mailbox being viewed. */
const FOLLOW = '__follow__'

export function ComposePane({ client }: { client: MailxClient }) {
  useTranslation()
  const UNDO_CHOICES: Record<string, string> = {
    '0': t('settings:undoOff'),
    '5': t('settings:undoFive'),
    '10': t('settings:undoTen'),
    '20': t('settings:undoTwenty'),
    '30': t('settings:undoThirty'),
  }
  const settings = useSettings(client)
  const update = useUpdateCompose(client)
  const accounts = useQuery({ queryKey: mailKeys.tree(), queryFn: () => loadMailTree(client), select: (tree) => tree.accounts })
  if (settings.isPending) return <Delayed><div className="flex justify-center p-10"><Spinner className="size-5" /></div></Delayed>
  if (settings.isError) return <SettingsPage>
    <Alert variant="destructive"><AlertTitle>{t('settings:readFailed')}</AlertTitle><AlertDescription>{errorText(settings.error)}</AlertDescription></Alert>
    <Button variant="outline" onClick={() => void settings.refetch()}>{t('settings:retry')}</Button>
  </SettingsPage>

  const compose = settings.data.settings.compose
  const list = accounts.data ?? []
  // An account that was deleted since reads as "follow"; the core also falls back to the first account.
  const accountValue = list.some((account) => account.id === compose.defaultAccountId) ? compose.defaultAccountId : FOLLOW
  const accountItems: Record<string, string> = { [FOLLOW]: t('settings:followMailbox'), ...Object.fromEntries(list.map((account) => [account.id, `${account.displayName}（${account.emailAddress}）`])) }

  return <SettingsPage>
    {update.isError && <Alert variant="destructive"><AlertTitle>{t('settings:saveFailedTitle')}</AlertTitle><AlertDescription>{errorText(update.error)}</AlertDescription></Alert>}
    <SettingsGroup title={t('settings:write')}>
      <SettingsRow label={t('settings:senderAccount')} htmlFor="compose.defaultAccount"
        description={t('settings:senderAccountDescription')}>
        <Select value={accountValue} items={accountItems}
          onValueChange={(value: string | null) => value && update.mutate({ defaultAccountId: value === FOLLOW ? '' : value })}>
          <SelectTrigger id="compose.defaultAccount" className="w-56"><SelectValue /></SelectTrigger>
          <SelectContent>
            {Object.entries(accountItems).map(([value, label]) => <SelectItem key={value} value={value}>{label}</SelectItem>)}
          </SelectContent>
        </Select>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title={t('settings:send')}>
      <SettingsRow label={t('settings:undo')} htmlFor="compose.undoSend"
        description={t('settings:undoDescription')}>
        <Select value={String(compose.undoSendSeconds)} items={UNDO_CHOICES}
          onValueChange={(value: string | null) => value && update.mutate({ undoSendSeconds: Number(value) })}>
          <SelectTrigger id="compose.undoSend" className="w-40"><SelectValue /></SelectTrigger>
          <SelectContent>
            {Object.entries(UNDO_CHOICES).map(([value, label]) => <SelectItem key={value} value={value}>{label}</SelectItem>)}
          </SelectContent>
        </Select>
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>
}
