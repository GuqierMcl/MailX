import { useEffect, useMemo, useState } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import {
  Cancel01Icon,
  Folder01Icon,
  InboxIcon,
  Mail01Icon,
  PaintBoardIcon,
  RefreshIcon,
  Search01Icon,
  Settings01Icon,
  SidebarLeftIcon,
  UserAdd01Icon,
} from '@hugeicons/core-free-icons'
import { useQuery } from '@tanstack/react-query'
import type { Account, EmailSummary, Folder, FolderRole, ThemeMode } from '@/generated/accounts'
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
import type { MailxClient } from '@/lib/mailx-client'
import { mailKeys } from '@/lib/query-client'
import { displayName, emailQuery, formatListTime } from './format'
import { folderLabel, type Mailbox } from './mailbox'

const RESULT_LIMIT = 8
const SEARCH_DELAY_MS = 200
const THEME_LABELS: Record<ThemeMode, string> = { system: '跟随系统', light: '浅色', dark: '深色' }

export type CommandPaletteProps = {
  client: MailxClient
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Names what a search covers, e.g. "工作邮箱 · 收件箱". */
  scope: string
  /** The search currently applied to the message list. */
  search: string
  folderId: string | null
  folderRole: FolderRole | null
  sourceIdentity: string
  accounts: Account[]
  folders: Folder[]
  onApplySearch: (text: string) => void
  /** Applies the search and selects the message once the list shows it. */
  onOpenEmail: (email: EmailSummary, text: string) => void
  onSelectMailbox: (mailbox: Mailbox) => void
  onOpenSettings: (target: SettingsTarget) => void
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
  client, onOpenChange, scope, search, folderId, folderRole, sourceIdentity, accounts, folders,
  onApplySearch, onOpenEmail, onSelectMailbox, onOpenSettings,
}: CommandPaletteProps) {
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

  const query = emailQuery({ folderId, folderRole, text: debounced, limit: RESULT_LIMIT })
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

  const jumps = useMemo<Entry[]>(() => [
    { id: 'jump:unified', title: '全部收件箱', icon: InboxIcon, run: () => onSelectMailbox({ kind: 'unified', role: 'inbox' }) },
    ...folders.filter((folder) => folder.selectable).map((folder): Entry => {
      const account = accounts.find((item) => item.id === folder.accountId)
      return {
        id: `jump:${folder.id}`,
        title: `${account?.displayName ?? ''} · ${folderLabel(folder)}`,
        keywords: `${folder.name} ${account?.emailAddress ?? ''}`,
        icon: folder.role === 'inbox' ? InboxIcon : Folder01Icon,
        run: () => onSelectMailbox({ kind: 'folder', folder }),
      }
    }),
  ], [accounts, folders, onSelectMailbox])

  const commands: Entry[] = [
    { id: 'cmd:settings', title: '打开设置', keywords: 'settings 偏好', icon: Settings01Icon, run: () => onOpenSettings({ section: 'appearance' }) },
    { id: 'cmd:add-account', title: '添加邮箱账号', keywords: 'account 新增', icon: UserAdd01Icon, run: () => onOpenSettings({ section: 'accounts', newAccount: true }) },
    ...(['light', 'dark', 'system'] as const).filter((mode) => mode !== theme).map((mode): Entry => ({
      id: `cmd:theme:${mode}`,
      title: `外观：${THEME_LABELS[mode]}`,
      keywords: 'theme 主题 明暗 暗色 深色 浅色 dark light',
      icon: PaintBoardIcon,
      run: () => appearance && updateAppearance.mutate({ ...appearance, theme: mode }),
    })),
    ...(accounts.length > 0 ? [{
      id: 'cmd:sync',
      title: '立即同步全部账号',
      keywords: 'sync refresh 刷新',
      icon: RefreshIcon,
      run: () => accounts.forEach((account) => void client.refreshSync(account.id).catch(() => undefined)),
    }] : []),
    ...(shell ? [{
      id: 'cmd:nav',
      title: shell.navOpen ? '收起侧边栏' : '展开侧边栏',
      keywords: 'sidebar',
      icon: SidebarLeftIcon,
      run: shell.toggleNav,
    }] : []),
    ...(search ? [{ id: 'cmd:clear-search', title: `清除搜索「${search}」`, keywords: 'clear', icon: Cancel01Icon, run: () => onApplySearch('') }] : []),
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

  const jumpGroup = shownJumps.length > 0 && <CommandGroup heading="跳转">{shownJumps.map(entryItem)}</CommandGroup>
  const commandGroup = shownCommands.length > 0 && <CommandGroup heading="命令">{shownCommands.map(entryItem)}</CommandGroup>

  return (
    <Command shouldFilter={false} loop label="命令面板">
      <CommandInput value={text} onValueChange={setText} placeholder={`在 ${scope} 中搜索，或输入命令…`} />
      <CommandList>
        {trimmed && (
          <CommandGroup heading="搜索">
            <CommandItem value="search:apply" onSelect={act(() => onApplySearch(trimmed))}>
              <HugeiconsIcon icon={Search01Icon} strokeWidth={2} />
              <span className="truncate">在 {scope} 中搜索「{trimmed}」</span>
            </CommandItem>
          </CommandGroup>
        )}
        {mails.length > 0 && (
          <CommandGroup heading={`邮件（${scope}）`}>
            {mails.map((email) => (
              <CommandItem key={email.id} value={`mail:${email.id}`} onSelect={act(() => onOpenEmail(email, trimmed))}>
                <HugeiconsIcon icon={Mail01Icon} strokeWidth={2} />
                <span className="w-24 shrink-0 truncate font-medium">{displayName(email.from)}</span>
                <span className="min-w-0 flex-1 truncate text-muted-foreground">{email.subject || '（无主题）'}</span>
                <span className="shrink-0 text-muted-foreground">{formatListTime(email.receivedAt)}</span>
              </CommandItem>
            ))}
          </CommandGroup>
        )}
        {/* With nothing typed the short command list comes first; the folder list can be long. */}
        {trimmed
          ? <>{jumpGroup}{commandGroup}</>
          : <>{commandGroup}{jumpGroup}</>}
        {nothingElse && <p role="status" className="px-3 pb-3 text-center text-xs text-muted-foreground">没有匹配的邮件、邮箱或命令</p>}
      </CommandList>
    </Command>
  )
}
