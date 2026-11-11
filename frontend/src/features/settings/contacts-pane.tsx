import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import { HugeiconsIcon } from '@hugeicons/react'
import { UserMultiple02Icon } from '@hugeicons/core-free-icons'
import type { ContactAggregation } from '@/generated/accounts'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Avatar, AvatarFallback } from '@/components/ui/avatar'
import { Button } from '@/components/ui/button'
import { Delayed } from '@/components/ui/delayed'
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { Spinner } from '@/components/ui/spinner'
import { toast } from '@/components/ui/toast'
import { avatarHue, formatListTime, initials } from '@/features/mail/format'
import type { MailxClient } from '@/lib/mailx-client'
import { contactKeys, mailKeys } from '@/lib/query-client'
import { SettingsGroup, SettingsPage } from './settings-layout'

/** Contact preferences. For now: whose mail the inbox collects into one row. */
export function ContactsPane({ client }: { client: MailxClient }) {
  useTranslation()
  const queryClient = useQueryClient()
  // The same query the message menu uses, so both always show the same rules.
  const rules = useQuery({ queryKey: contactKeys.aggregations(), queryFn: () => client.listContactAggregations() })
  const ungroup = useMutation({
    mutationFn: (address: string) => client.setContactAggregation(address, false),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: contactKeys.aggregations() })
      void queryClient.invalidateQueries({ queryKey: mailKeys.emailLists() })
    },
    onError: (error) => toast.error(t('settings:contactsUngroupFailed'), { description: errorText(error) }),
  })

  if (rules.isPending) return <Delayed><div className="flex justify-center p-10"><Spinner className="size-5" /></div></Delayed>
  if (rules.isError) return <SettingsPage>
    <Alert variant="destructive"><AlertTitle>{t('settings:contactsLoadFailed')}</AlertTitle><AlertDescription>{errorText(rules.error)}</AlertDescription></Alert>
    <Button variant="outline" onClick={() => void rules.refetch()}>{t('settings:retry')}</Button>
  </SettingsPage>

  return <SettingsPage>
    <SettingsGroup title={t('settings:contactsGroupedTitle')} description={rules.data.length > 0 ? t('settings:contactsGroupedDescription') : undefined}>
      {rules.data.length === 0 ? (
        <Empty className="py-10">
          <EmptyHeader>
            <EmptyMedia variant="icon"><HugeiconsIcon icon={UserMultiple02Icon} /></EmptyMedia>
            <EmptyTitle>{t('settings:contactsEmptyTitle')}</EmptyTitle>
            <EmptyDescription>{t('settings:contactsGroupedDescription')}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <ul className="-mx-2 flex flex-col">
          {rules.data.map((rule) => (
            <ContactRow key={rule.address} rule={rule} busy={ungroup.isPending && ungroup.variables === rule.address} onUngroup={() => ungroup.mutate(rule.address)} />
          ))}
        </ul>
      )}
    </SettingsGroup>
  </SettingsPage>
}

function ContactRow({ rule, busy, onUngroup }: { rule: ContactAggregation; busy: boolean; onUngroup: () => void }) {
  const name = rule.name?.trim() || null
  const label = name ?? rule.address
  return (
    <li className="flex items-center gap-3 rounded-lg px-2 py-2">
      <Avatar>
        <AvatarFallback className="text-white" style={{ backgroundColor: `hsl(${avatarHue(rule.address)} 45% 42%)` }}>
          {initials({ name, address: rule.address })}
        </AvatarFallback>
      </Avatar>
      <div className="flex min-w-0 flex-1 flex-col">
        <span className="flex min-w-0 items-baseline gap-2 text-sm">
          <span className="truncate font-medium" title={label}>{label}</span>
          {name && <span className="min-w-0 truncate text-xs text-muted-foreground" title={rule.address}>{rule.address}</span>}
        </span>
        <span className="truncate text-xs text-muted-foreground">
          {rule.latestReceivedAt
            ? t('settings:contactsMessageSummary', { count: rule.messageCount, date: formatListTime(rule.latestReceivedAt) })
            : t('settings:contactsMessageCount', { count: rule.messageCount })}
        </span>
      </div>
      <Button variant="outline" size="sm" disabled={busy} aria-label={t('settings:contactsUngroupLabel', { name: label })} onClick={onUngroup}>
        {t('settings:contactsUngroup')}
      </Button>
    </li>
  )
}
