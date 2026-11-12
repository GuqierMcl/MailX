import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { useEffect, useState } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import {
  Archive03Icon,
  AtSignIcon,
  Cancel01Icon,
  Delete02Icon,
  File01Icon,
  Folder01Icon,
  InboxIcon,
  LanguagesIcon,
  Mail01Icon,
  PaintBoardIcon,
  PencilEdit02Icon,
  RefreshIcon,
  Search01Icon,
  SentIcon,
  Settings01Icon,
  SidebarLeftIcon,
  SpamIcon,
  StarIcon,
  UserAdd01Icon,
} from '@hugeicons/core-free-icons'
import { useQuery } from '@tanstack/react-query'
import type { Account, EmailListQuery, EmailSummary, Folder, ThemeMode } from '@/generated/accounts'
import {
  Command,
  CommandDialog,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
} from '@/components/ui/command'
import { useShell } from '@/features/shell/shell-context'
import { useSettings, useUpdateAppearance } from '@/features/settings/settings-queries'
import type { SettingsTarget } from '@/features/settings/settings-dialog'
import type { EmailTranslation } from './translation/use-email-translation'
import type { MailxClient } from '@/lib/mailx-client'
import { mailKeys } from '@/lib/query-client'
import { displayName, emailQuery, formatListTime } from './format'
import { folderLabel, type Mailbox } from './mailbox'

/** The core rejects longer search text. */
const SEARCH_MAX_CHARS = 256

const RESULT_LIMIT = 8
const SEARCH_DELAY_MS = 200
const THEME_LABELS: Record<ThemeMode, string> = { get system() { return t('mail:systemTheme') }, get light() { return t('mail:lightTheme') }, get dark() { return t('mail:darkTheme') } }

export type CommandPaletteProps = {
  client: MailxClient
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Names what a search covers, e.g. "工作邮箱 · 收件箱". */
  scope: string
  /** The search currently applied to the message list. */
  search: string
  /** The current list's scope (folder, role, archive, starred), without its text or unread filter. */
  scopeQuery: Partial<EmailListQuery>
  sourceIdentity: string
  accounts: Account[]
  folders: Folder[]
  onApplySearch: (text: string) => void
  /** Applies the search and selects the message once the list shows it. */
  onOpenEmail: (email: EmailSummary, text: string) => void
  onSelectMailbox: (mailbox: Mailbox) => void
  onOpenAccounts: () => void
  onAddAccount: () => void
  onOpenSettings: (target: SettingsTarget) => void
  onCompose: () => void
  /** Translation of the open message; null when no translatable message is on screen. */
  translation?: EmailTranslation | null
}

type Entry = { id: string; title: string; keywords?: string; icon: Parameters<typeof HugeiconsIcon>[0]['icon']; run: () => void }

const matches = (text: string, entry: Pick<Entry, 'title' | 'keywords'>) => {
  const haystack = `${entry.title} ${entry.keywords ?? ''}`.toLowerCase()
  return text.toLowerCase().split(/\s+/).filter(Boolean).every((word) => haystack.includes(word))
}

export function CommandPalette(props: CommandPaletteProps) {
  return (
    <CommandDialog open={props.open} onOpenChange={props.onOpenChange}>
      {/* Mounted only while open, so every opening starts with an empty input. */}
      <PaletteBody {...props} />
    </CommandDialog>
  )
}

function PaletteBody({
  client, onOpenChange, scope, search, scopeQuery, sourceIdentity, accounts, folders,
  onApplySearch, onOpenEmail, onSelectMailbox, onOpenAccounts, onAddAccount, onOpenSettings, onCompose, translation,
}: CommandPaletteProps) {
  useTranslation()
  const [text, setText] = useState('')
  const [debounced, setDebounced] = useState('')
  const shell = useShell()
  const settings = useSettings(client)
  const updateAppearance = useUpdateAppearance(client)
  const theme = settings.data?.settings.appearance.theme
  const appearance = settings.data?.settings.appearance

  useEffect(() => {
    const timer = setTimeout(() => setDebounced(text.trim()), SEARCH_DELAY_MS)
    return () => clearTimeout(timer)
  }, [text])

  const query = emailQuery({ ...scopeQuery, text: debounced, limit: RESULT_LIMIT })
  const results = useQuery({
    queryKey: [...mailKeys.all, 'palette', sourceIdentity, query],
    queryFn: () => client.listEmails(query),
    enabled: debounced !== '',
    placeholderData: (previous) => previous,
  })

  const close = () => onOpenChange(false)
  const act = (run: () => void) => () => {
    close()
    run()
  }

  const accountPosition = new Map(accounts.map((account, index) => [account.id, index]))
  const orderedFolders = [...folders].sort((left, right) =>
    (accountPosition.get(left.accountId) ?? accounts.length) - (accountPosition.get(right.accountId) ?? accounts.length))
  // The same views, in the same order, as the top of the sidebar.
  const views: [string, string, Entry['icon'], Mailbox][] = [
    ['inbox', t('mail:allInboxes'), InboxIcon, { kind: 'unified', role: 'inbox' }],
    ['starred', t('mail:starredView'), StarIcon, { kind: 'starred' }],
    ['drafts', t('mail:drafts'), File01Icon, { kind: 'drafts' }],
    ['sent', t('mail:allSent'), SentIcon, { kind: 'unified', role: 'sent' }],
    ['archived', t('mail:allArchived'), Archive03Icon, { kind: 'archived', accountId: null }],
    ['junk', t('mail:allJunk'), SpamIcon, { kind: 'unified', role: 'junk' }],
    ['trash', t('mail:allTrash'), Delete02Icon, { kind: 'unified', role: 'trash' }],
  ]
  const jumps: Entry[] = [
    ...views.map(([id, title, icon, mailbox]): Entry => ({ id: `jump:view:${id}`, title, icon, run: () => onSelectMailbox(mailbox) })),
    ...orderedFolders.filter((folder) => folder.selectable).map((folder): Entry => {
      const account = accounts.find((item) => item.id === folder.accountId)
      return {
        id: `jump:${folder.id}`,
        title: `${account?.displayName ?? ''} · ${folderLabel(folder)}`,
        keywords: `${folder.name} ${account?.emailAddress ?? ''}`,
        icon: folder.role === 'inbox' ? InboxIcon : Folder01Icon,
        run: () => onSelectMailbox({ kind: 'folder', folder }),
      }
    }),
  ]

  const translationCommand: Entry[] = !translation ? [] : [translation.translationVisible
    ? { id: 'cmd:translation', title: t('mail:translationShowOriginal'), keywords: 'translate translation original 翻译 原文', icon: LanguagesIcon, run: translation.showOriginal }
    : translation.status?.translation
      ? { id: 'cmd:translation', title: t('mail:translationShowTranslation'), keywords: 'translate translation 翻译 译文', icon: LanguagesIcon, run: translation.showTranslation }
      : {
        id: 'cmd:translation', title: t('mail:translationCommand'), keywords: 'translate translation 翻译', icon: LanguagesIcon,
        // Without a service there is nothing to run: go to where it is set up.
        run: () => translation.configured ? translation.translate() : onOpenSettings({ section: 'translation' }),
      }]
  const commands: Entry[] = [
    ...translationCommand,
    ...(accounts.length > 0 ? [{ id: 'cmd:compose', title: t('mail:compose'), keywords: 'compose new mail 新邮件 撰写 发信', icon: PencilEdit02Icon, run: onCompose }] : []),
    { id: 'cmd:settings', title: t('mail:openSettings'), keywords: 'settings 偏好', icon: Settings01Icon, run: () => onOpenSettings({ section: 'general' }) },
    { id: 'cmd:accounts', title: t('mail:manageAccounts'), keywords: 'account 账号 邮箱', icon: AtSignIcon, run: onOpenAccounts },
    { id: 'cmd:add-account', title: t('mail:addAccount'), keywords: 'account 新增', icon: UserAdd01Icon, run: onAddAccount },
    ...(['light', 'dark', 'system'] as const).filter((mode) => mode !== theme).map((mode): Entry => ({
      id: `cmd:theme:${mode}`,
      title: t('mail:appearanceCommand', { theme: THEME_LABELS[mode] }),
      keywords: 'theme 主题 明暗 暗色 深色 浅色 dark light',
      icon: PaintBoardIcon,
      run: () => appearance && updateAppearance.mutate({ ...appearance, theme: mode }),
    })),
    ...(accounts.length > 0 ? [{
      id: 'cmd:sync',
      title: t('mail:syncAllAccounts'),
      keywords: 'sync refresh 刷新',
      icon: RefreshIcon,
      run: () => accounts.forEach((account) => void client.refreshSync(account.id).catch(() => undefined)),
    }] : []),
    ...(shell ? [{
      id: 'cmd:nav',
      title: shell.navOpen ? t('mail:collapseSidebar') : t('mail:expandSidebar'),
      keywords: 'sidebar',
      icon: SidebarLeftIcon,
      run: shell.toggleNav,
    }] : []),
    ...(search ? [{ id: 'cmd:clear-search', title: t('mail:clearSearchTerm', { search }), keywords: 'clear', icon: Cancel01Icon, run: () => onApplySearch('') }] : []),
  ]

  const trimmed = text.trim()
  const shownJumps = jumps.filter((entry) => matches(trimmed, entry))
  const shownCommands = commands.filter((entry) => matches(trimmed, entry))
  const mails = trimmed && debounced ? results.data?.items ?? [] : []
  // The search item is always offered, so this only reports that nothing else matched.
  const nothingElse = trimmed !== '' && debounced === trimmed && !results.isFetching &&
    mails.length === 0 && shownJumps.length === 0 && shownCommands.length === 0

  const entryItem = (entry: Entry) => (
    <CommandItem key={entry.id} value={entry.id} onSelect={act(entry.run)}>
      <HugeiconsIcon icon={entry.icon} strokeWidth={2} />
      <span className="truncate">{entry.title}</span>
    </CommandItem>
  )

  const jumpGroup = shownJumps.length > 0 && <CommandGroup heading={t('mail:jumpTo')}>{shownJumps.map(entryItem)}</CommandGroup>
  const commandGroup = shownCommands.length > 0 && <CommandGroup heading={t('mail:commands')}>{shownCommands.map(entryItem)}</CommandGroup>

  return (
    <Command shouldFilter={false} loop label={t('mail:commandPalette')}>
      <CommandInput maxLength={SEARCH_MAX_CHARS} value={text} onValueChange={setText} placeholder={t('mail:searchOrCommand', { scope })} />
      <CommandList>
        {trimmed && (
          <CommandGroup heading={t('mail:search')}>
            <CommandItem value="search:apply" onSelect={act(() => onApplySearch(trimmed))}>
              <HugeiconsIcon icon={Search01Icon} strokeWidth={2} />
              <span className="truncate">{t('mail:searchTermInScope', { scope, query: trimmed })}</span>
            </CommandItem>
          </CommandGroup>
        )}
        {mails.length > 0 && (
          <CommandGroup heading={t('mail:messagesInScope', { scope })}>
            {mails.map((email) => (
              <CommandItem key={email.id} value={`mail:${email.id}`} onSelect={act(() => onOpenEmail(email, trimmed))}>
                <HugeiconsIcon icon={Mail01Icon} strokeWidth={2} />
                <span className="w-24 shrink-0 truncate font-medium">{displayName(email.from)}</span>
                <span className="min-w-0 flex-1 truncate text-muted-foreground">{email.subject || t('mail:noSubject')}</span>
                <span className="shrink-0 text-muted-foreground">{formatListTime(email.receivedAt)}</span>
              </CommandItem>
            ))}
          </CommandGroup>
        )}
        {/* With nothing typed the short command list comes first; the folder list can be long. */}
        {trimmed
          ? <>{jumpGroup}{commandGroup}</>
          : <>{commandGroup}{jumpGroup}</>}
        {nothingElse && <p role="status" className="px-3 pb-3 text-center text-xs text-muted-foreground">{t('mail:noMatches')}</p>}
      </CommandList>
    </Command>
  )
}
