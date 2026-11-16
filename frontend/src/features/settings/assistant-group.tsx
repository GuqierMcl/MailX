import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { HugeiconsIcon } from '@hugeicons/react'
import { ArrowDown01Icon } from '@hugeicons/core-free-icons'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { CommandGroup, CommandItem } from '@/components/ui/command'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Switch } from '@/components/ui/switch'
import { Textarea } from '@/components/ui/textarea'
import { AccountAvatar } from '@/features/accounts/account-avatar'
import { sortAccountsForDisplay } from '@/features/accounts/account-order'
import { loadMailTree } from '@/features/mail/mail-tree'
import type { AiModelRef, AiState, AssistantMode, AssistantSendMode, AssistantSettings } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'
import { mailKeys, settingsKeys } from '@/lib/query-client'
import { ModelList } from './ai-model-picker'
import { useModelGroups } from './ai-queries'
import { SettingsGroup, SettingsRow } from './settings-layout'
import { useSettings, useUpdateAssistant } from './settings-queries'

/** What the core accepts for `assistant.instructions`. */
export const INSTRUCTIONS_MAX = 2000
/** The counter shows once the text is within this many characters of the limit. */
const COUNTER_FROM = INSTRUCTIONS_MAX - 200

type Update = ReturnType<typeof useUpdateAssistant>

const hostOf = (baseUrl: string) => {
  try { return new URL(baseUrl).host } catch { return baseUrl }
}

/**
 * Settings → Integrations → AI → Assistant (ai-runtime §7). Everything except the consent goes
 * through the normal settings write; the consent has its own commands and is only revoked here.
 * `chat` shows the rows only the chat uses (what the assistant may do, custom instructions); the
 * app turns it on now that the chat exists, so no setting appears that changes nothing.
 */
export function AssistantGroup({ client, ai, chat = false }: { client: MailxClient; ai: AiState; chat?: boolean }) {
  useTranslation()
  const settings = useSettings(client)
  const update = useUpdateAssistant(client)
  const assistant = settings.data?.settings.assistant
  if (!assistant) return null
  const off = !assistant.enabled
  return <SettingsGroup title={t('settings:assistant')}>
    {update.isError && <Alert variant="destructive"><AlertTitle>{t('settings:saveFailedTitle')}</AlertTitle><AlertDescription>{errorText(update.error)}</AlertDescription></Alert>}
    <SettingsRow label={t('settings:assistantEnabled')} description={t('settings:assistantEnabledDescription')} htmlFor="assistant.enabled">
      <Switch id="assistant.enabled" checked={assistant.enabled} onCheckedChange={(enabled) => update.mutate({ enabled })} />
    </SettingsRow>
    <ModelRow client={client} ai={ai} assistant={assistant} off={off} update={update} />
    <ConsentRow client={client} ai={ai} assistant={assistant} off={off} />
    <AccountsRow client={client} assistant={assistant} off={off} update={update} />
    {/* Only the chat uses these. */}
    {chat && <>
      <ModeRow id="assistant.organize" label={t('settings:assistantOrganize')} description={t('settings:assistantOrganizeDescription')}
        value={assistant.organize} off={off} onChange={(organize) => update.mutate({ organize })} />
      <ModeRow id="assistant.aiDrafts" label={t('settings:assistantAiDrafts')} description={t('settings:assistantAiDraftsDescription')}
        value={assistant.aiDrafts} off={off} onChange={(aiDrafts) => update.mutate({ aiDrafts })} />
      <SendRow assistant={assistant} off={off} update={update} />
      <InstructionsRow key={assistant.instructions} assistant={assistant} off={off} update={update} />
    </>}
  </SettingsGroup>
}

/** Which model the assistant uses: its own choice, or whatever the default model is. */
function ModelRow({ client, ai, assistant, off, update }: { client: MailxClient; ai: AiState; assistant: AssistantSettings; off: boolean; update: Update }) {
  useTranslation()
  const [open, setOpen] = useState(false)
  const groups = useModelGroups(client, ai, open)
  const follow = ai.defaultModel ? t('settings:assistantModelFollow', { name: ai.defaultModel.name }) : t('settings:assistantModelFollowBare')
  const choose = (model: AiModelRef | null) => {
    setOpen(false)
    update.mutate({ model })
  }
  const connected = ai.connections.length > 0
  return <SettingsRow label={t('settings:assistantModel')} htmlFor="assistant.model" description={t('settings:assistantModelDescription')}>
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger render={<Button id="assistant.model" role="combobox" aria-label={t('settings:assistantModel')} aria-expanded={open} variant="outline" className="w-56 justify-between font-normal" disabled={off || !connected} />}>
        <span className="truncate">{assistant.model ? assistant.model.name : follow}</span>
        <HugeiconsIcon icon={ArrowDown01Icon} strokeWidth={2} className="size-4 shrink-0 text-muted-foreground" />
      </PopoverTrigger>
      <PopoverContent align="end" className="w-80 gap-0 p-0">
        <ModelList client={client} groups={groups} onPick={choose} leading={
          <CommandGroup>
            <CommandItem value="follow-default" onSelect={() => choose(null)}>{follow}</CommandItem>
          </CommandGroup>
        } />
      </PopoverContent>
    </Popover>
  </SettingsRow>
}

/** Consent is asked where it is first needed; here it can only be taken back. */
function ConsentRow({ client, ai, assistant, off }: { client: MailxClient; ai: AiState; assistant: AssistantSettings; off: boolean }) {
  useTranslation()
  const queryClient = useQueryClient()
  const [failure, setFailure] = useState<string | null>(null)
  const { consented } = assistant
  const service = consented ? ai.connections.find((connection) => connection.id === consented.connectionId)?.name ?? hostOf(consented.baseUrl) : null
  const revoke = () => {
    setFailure(null)
    client.assistantRevokeConsent().then(
      () => queryClient.invalidateQueries({ queryKey: settingsKeys.all }),
      (error: unknown) => setFailure(errorText(error)),
    )
  }
  return <SettingsRow label={t('settings:assistantConsent')}>
    <div className="flex flex-col items-end gap-1">
      <div className="flex items-center gap-3">
        <span className="text-sm text-muted-foreground">{service ? t('settings:assistantConsentGiven', { service }) : t('settings:assistantConsentNone')}</span>
        {consented && <Button variant="outline" size="sm" disabled={off} onClick={revoke}>{t('settings:assistantConsentRevoke')}</Button>}
      </div>
      {failure && <span role="alert" className="max-w-72 text-right text-xs text-destructive">{failure}</span>}
    </div>
  </SettingsRow>
}

/** Unticking a mailbox hides it from the assistant; a ticked one is simply not in the list. */
function AccountsRow({ client, assistant, off, update }: { client: MailxClient; assistant: AssistantSettings; off: boolean; update: Update }) {
  useTranslation()
  const settings = useSettings(client)
  const accounts = useQuery({ queryKey: mailKeys.tree(), queryFn: () => loadMailTree(client), select: (tree) => tree.accounts })
  const list = sortAccountsForDisplay(accounts.data ?? [], settings.data?.settings.layout.accountOrder ?? [])
  if (list.length === 0) return null
  const excluded = new Set(assistant.excludedAccounts)
  const toggle = (id: string, allowed: boolean) => {
    const next = new Set(excluded)
    if (allowed) next.delete(id)
    else next.add(id)
    update.mutate({ excludedAccounts: [...next].sort() })
  }
  return <SettingsRow label={t('settings:assistantAccounts')} description={t('settings:assistantAccountsDescription')}>
    <div role="group" aria-label={t('settings:assistantAccounts')} className="flex flex-col items-start gap-2">
      {list.map((account) => (
        <label key={account.id} className="flex min-w-0 items-center gap-2 text-sm">
          <Checkbox checked={!excluded.has(account.id)} disabled={off} aria-label={account.displayName}
            onCheckedChange={(allowed) => toggle(account.id, allowed)} />
          <AccountAvatar displayName={account.displayName} color={account.color} providerId={account.providerId} useProviderIcon={account.useProviderIcon} className="size-[18px] text-[10px]" />
          <span className="truncate">{account.displayName}</span>
        </label>
      ))}
    </div>
  </SettingsRow>
}

function ModeRow({ id, label, description, value, off, onChange }: { id: string; label: string; description: string; value: AssistantMode; off: boolean; onChange: (mode: AssistantMode) => void }) {
  useTranslation()
  const items: Record<AssistantMode, string> = { direct: t('settings:assistantModeDirect'), ask: t('settings:assistantModeAsk') }
  return <SettingsRow label={label} description={description} htmlFor={id}>
    <Select value={value} items={items} disabled={off} onValueChange={(mode: AssistantMode | null) => mode && onChange(mode)}>
      <SelectTrigger id={id} className="w-36"><SelectValue /></SelectTrigger>
      <SelectContent>{Object.entries(items).map(([mode, text]) => <SelectItem key={mode} value={mode}>{text}</SelectItem>)}</SelectContent>
    </Select>
  </SettingsRow>
}

/** Sending is the one choice that needs a reminder of what it means. */
function SendRow({ assistant, off, update }: { assistant: AssistantSettings; off: boolean; update: Update }) {
  useTranslation()
  const items: Record<AssistantSendMode, string> = { confirm: t('settings:assistantSendConfirm'), direct: t('settings:assistantSendDirect') }
  return <SettingsRow label={t('settings:assistantSend')} htmlFor="assistant.send">
    <div className="flex flex-col items-end gap-1.5">
      <Select value={assistant.send} items={items} disabled={off} onValueChange={(send: AssistantSendMode | null) => send && update.mutate({ send })}>
        <SelectTrigger id="assistant.send" className="w-36"><SelectValue /></SelectTrigger>
        <SelectContent>{Object.entries(items).map(([mode, text]) => <SelectItem key={mode} value={mode}>{text}</SelectItem>)}</SelectContent>
      </Select>
      {assistant.send === 'direct' && <p className="max-w-56 text-right text-xs text-muted-foreground">{t('settings:assistantSendDirectHint')}</p>}
    </div>
  </SettingsRow>
}

/** Saved when the field loses focus, not on every key. The counter appears only near the limit. */
function InstructionsRow({ assistant, off, update }: { assistant: AssistantSettings; off: boolean; update: Update }) {
  useTranslation()
  const [value, setValue] = useState(assistant.instructions)
  const length = Array.from(value).length
  const commit = () => { if (value !== assistant.instructions) update.mutate({ instructions: value }) }
  return <SettingsRow label={t('settings:assistantInstructions')} description={t('settings:assistantInstructionsDescription')} htmlFor="assistant.instructions">
    <div className="flex w-64 flex-col items-end gap-1">
      <Textarea id="assistant.instructions" className="min-h-24 resize-y" value={value} disabled={off} maxLength={INSTRUCTIONS_MAX} spellCheck={false}
        onChange={(event) => setValue(event.target.value)} onBlur={commit} />
      {length >= COUNTER_FROM && <span className="text-xs text-muted-foreground tabular-nums">{t('settings:assistantInstructionsCount', { length, max: INSTRUCTIONS_MAX })}</span>}
    </div>
  </SettingsRow>
}
