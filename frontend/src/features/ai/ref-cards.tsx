import { useContext } from 'react'
import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { t } from '@/i18n'
import type { Account, EmailSummary, Folder } from '@/generated/accounts'
import { AccountAvatar } from '@/features/accounts/account-avatar'
import { displayName } from '@/features/mail/format'
import { folderLabel } from '@/features/mail/mailbox'
import { SenderAvatar } from '@/features/mail/sender-avatar'
import { ROLE_ICONS } from '@/features/mail/sidebar-shared'
import { useMinuteClock } from '@/features/now/visit'
import { MailRowCard, rowClass } from '@/features/now/mail-row-card'
import { cn } from '@/lib/utils'
import type { RefKind } from './markdown'
import { ChatEnvContext, type ChatEnv } from './chat-env'
import { useAssistantEmail } from './use-assistant-email'

type RefProps = { id: string; label: string; block: boolean }

/** What a reference falls back to when its target is not here: the model's own words, no link. */
function Plain({ label, block }: { label: string; block: boolean }) {
  return block ? <p>{label}</p> : <>{label}</>
}

const chipClass = 'inline-flex max-w-64 items-center gap-1 rounded-full bg-muted/60 py-px pr-2 pl-1 align-middle text-[0.875em] leading-snug text-foreground outline-none hover:bg-foreground/8 focus-visible:ring-2 focus-visible:ring-ring'

function EmailRef({ env, id, label, block }: RefProps & { env: ChatEnv }) {
  const { i18n } = useTranslation()
  const email = useAssistantEmail(env.client, id)
  const minute = useMinuteClock()
  const summary: EmailSummary | undefined = email.data?.summary
  if (!summary) {
    // Still loading: a block holds its place so the reply does not jump; a chip is just its words.
    if (email.isPending && block) return <div aria-hidden className="h-11 rounded-lg bg-muted/40" />
    return <Plain label={label} block={block} />
  }
  const open = () => env.onOpenEmail({ id: summary.id, accountId: summary.accountId })
  const name = t('ai:chat.openEmailNamed', { subject: summary.subject || t('mail:noSubject') })
  if (block) {
    return <MailRowCard email={summary} language={i18n.language} now={minute} unread={!summary.isRead} onOpen={open} label={name} />
  }
  return (
    <button type="button" aria-label={name} onClick={open} className={chipClass}>
      <SenderAvatar sender={summary.from} accountId={summary.accountId} size="sm" className="size-3.5 text-[7px]" />
      <span className="truncate">{summary.subject || displayName(summary.from)}</span>
    </button>
  )
}

function FolderRef({ env, id, label, block }: RefProps & { env: ChatEnv }) {
  useTranslation()
  const folder: Folder | undefined = env.folders.find((item) => item.id === id && item.selectable)
  if (!folder) return <Plain label={label} block={block} />
  const account: Account | undefined = env.accounts.find((item) => item.id === folder.accountId)
  const name = folderLabel(folder)
  const unread = folder.role === 'junk' ? 0 : folder.unreadCount
  const open = () => env.onOpenFolder(folder)
  const aria = t('ai:chat.openFolderNamed', { name })
  if (block) {
    return (
      <button type="button" aria-label={aria} onClick={open} className={rowClass}>
        <HugeiconsIcon icon={ROLE_ICONS[folder.role]} aria-hidden strokeWidth={2} className="size-5 shrink-0 text-muted-foreground" />
        <span className="min-w-0 flex-1 truncate font-medium">{name}</span>
        {account && (
          <span className="flex min-w-0 shrink items-center gap-1.5 text-xs text-muted-foreground">
            <AccountAvatar displayName={account.displayName} color={account.color} providerId={account.providerId} useProviderIcon={account.useProviderIcon} className="size-4 text-[9px]" />
            <span className="truncate">{account.displayName}</span>
          </span>
        )}
        {folder.countsKnown && unread > 0 && <span className="shrink-0 text-xs text-muted-foreground tabular-nums" title={t('ai:chat.folderUnread', { count: unread })}>{unread}</span>}
      </button>
    )
  }
  return (
    <button type="button" aria-label={aria} onClick={open} className={cn(chipClass, 'pl-1.5')}>
      <HugeiconsIcon icon={ROLE_ICONS[folder.role]} aria-hidden strokeWidth={2} className="size-3.5 shrink-0 text-muted-foreground" />
      <span className="truncate">{name}</span>
    </button>
  )
}

/**
 * A `mailx://` link in a reply (ai-chat-ui.md 4.2): a card when it stands alone on its line, a small chip
 * inside a sentence. The words come from local data, never from the model's label, which is only the
 * fallback when the mail or folder is not here. Outside a chat there is nothing to open, so it is plain text.
 */
export function RefView({ kind, id, label, block }: { kind: RefKind } & RefProps) {
  const env = useContext(ChatEnvContext)
  if (!env) return <Plain label={label} block={block} />
  return kind === 'email'
    ? <EmailRef env={env} id={id} label={label} block={block} />
    : <FolderRef env={env} id={id} label={label} block={block} />
}
