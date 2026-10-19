import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import { Add01Icon, Mail01Icon } from '@hugeicons/core-free-icons'
import { useInfiniteQuery, useQuery, useQueryClient } from '@tanstack/react-query'
import type { AccountSyncState } from '@/generated/accounts'
import { Button } from '@/components/ui/button'
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/components/ui/resizable'
import { SidebarInset, SidebarProvider, SidebarTrigger } from '@/components/ui/sidebar'
import { TooltipProvider } from '@/components/ui/tooltip'
import { clientFailure, type MailxClient } from '@/lib/mailx-client'
import { mailKeys } from '@/lib/query-client'
import { EmailList, type ListFilter } from './email-list'
import { EmailView } from './email-view'
import { emailQuery } from './format'
import { TitleBarActions } from '@/features/window-chrome/title-bar'
import { SettingsDialog, type SettingsTarget } from '@/features/settings/settings-dialog'
import { MailSidebar } from './mail-sidebar'
import { mailboxTitle, type Mailbox } from './mailbox'
import { acceptsSyncState, folderSyncStatus } from './folder-sync-state'

const SEARCH_DELAY_MS = 200
const TREE_RELOAD_DELAY_MS = 250
const PAGE_SIZE = 100

export function Workbench({ client }: { client: MailxClient }) {
  const queryClient = useQueryClient()
  const [settings, setSettings] = useState<SettingsTarget | null>(null)
  const [mailbox, setMailbox] = useState<Mailbox>({ kind: 'unified', role: 'inbox' })
  const [filter, setFilter] = useState<ListFilter>('all')
  const [search, setSearch] = useState('')
  const [query, setQuery] = useState('')
  const [syncStates, setSyncStates] = useState<Record<string, AccountSyncState>>({})
  const syncSnapshots = useRef<Record<string, AccountSyncState>>({})
  const [selectedId, setSelectedId] = useState<string | null>(null)

  const tree = useQuery({
    queryKey: mailKeys.tree(),
    queryFn: async () => {
      const accounts = await client.listAccounts()
      const folders = (await Promise.all(accounts.map((account) => client.listFolders(account.id)))).flat()
      return { accounts, folders }
    },
  })
  const treeLoading = tree.isPending
  const accounts = useMemo(() => tree.data?.accounts ?? [], [tree.data])
  const folders = useMemo(() => tree.data?.folders ?? [], [tree.data])

  // One sync round can change many folders; reload the tree once for the burst.
  // The timer outlives the subscription effect below: snapshots are already
  // updated when it is scheduled, so dropping it on re-subscribe would lose the reload.
  const treeReloadTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  const scheduleTreeReload = useCallback(() => {
    clearTimeout(treeReloadTimer.current)
    treeReloadTimer.current = setTimeout(
      () => void queryClient.invalidateQueries({ queryKey: mailKeys.tree() }),
      TREE_RELOAD_DELAY_MS,
    )
  }, [queryClient])
  useEffect(() => () => clearTimeout(treeReloadTimer.current), [])

  const accountIdentity = JSON.stringify(accounts.map(({ id, mailboxSourceId }) => [id, mailboxSourceId]).sort())
  useEffect(() => {
    let active = true
    const currentAccounts = new Map<string, string>(JSON.parse(accountIdentity))
    const apply = (state: AccountSyncState) => {
      if (!active) return
      const source = currentAccounts.get(state.accountId)
      const previous = syncSnapshots.current[state.accountId]
      if (!source || !acceptsSyncState({ mailboxSourceId: source }, previous, state)) return
      const previousGeneration = previous?.mailboxSourceId === state.mailboxSourceId && previous.generation === state.generation
        ? previous : undefined
      syncSnapshots.current = { ...syncSnapshots.current, [state.accountId]: state }
      setSyncStates(syncSnapshots.current)
      // A queried snapshot also recovers notifications missed while the tree was loading.
      // Revision zero is the initial baseline, so an unchanged snapshot cannot loop.
      if (state.folderRevision !== (previousGeneration?.folderRevision ?? 0) ||
        state.folders.some((folder) => folder.dataRevision !== (previousGeneration?.folders.find((item) => item.folderId === folder.folderId)?.dataRevision ?? 0))) {
        scheduleTreeReload()
        // Cached lists for other mailboxes may now be out of date. Mark them
        // stale without refetching; each refetches when shown again. The list on
        // screen refreshes through its own scope check below.
        void queryClient.invalidateQueries({ queryKey: mailKeys.emailLists(), refetchType: 'none' })
      }
    }
    const stop = client.onSyncState(apply)
    client.listSyncStates().then(
      (states) => active && states.forEach((state) => apply(state)),
      () => undefined,
    )
    return () => {
      active = false
      stop()
    }
  }, [client, accountIdentity, scheduleTreeReload, queryClient])

  useEffect(() => {
    const timer = setTimeout(() => setQuery(search.trim()), SEARCH_DELAY_MS)
    return () => clearTimeout(timer)
  }, [search])

  const currentFolder = mailbox.kind === 'folder' ? folders.find((folder) => folder.id === mailbox.folder.id && folder.selectable) : null
  const activeMailbox: Mailbox = mailbox.kind === 'folder' && !treeLoading
    ? currentFolder ? { kind: 'folder', folder: currentFolder } : { kind: 'unified', role: 'inbox' }
    : mailbox
  const folderId = activeMailbox.kind === 'folder' ? activeMailbox.folder.id : null
  const folderRole = activeMailbox.kind === 'unified' ? activeMailbox.role : null
  const selectedAccountId = activeMailbox.kind === 'folder' ? activeMailbox.folder.accountId : null
  const sourceIdentity = JSON.stringify(accounts.filter((account) => !selectedAccountId || account.id === selectedAccountId)
    .map((account) => [account.id, account.mailboxSourceId]).sort())
  const currentSyncStates = Object.fromEntries(accounts.flatMap((account) => {
    const state = syncStates[account.id]
    return state?.mailboxSourceId === account.mailboxSourceId ? [[account.id, state]] : []
  }))
  const scopedFolders = folders.filter((folder) => folder.selectable && (folderId ? folder.id === folderId : folder.role === folderRole))
  const scopeRevision = JSON.stringify(scopedFolders.map((folder) => [
    folder.id,
    accounts.find((account) => account.id === folder.accountId)?.mailboxSourceId,
    currentSyncStates[folder.accountId]?.generation ?? 0,
    currentSyncStates[folder.accountId]?.folders.find((state) => state.folderId === folder.id)?.dataRevision ?? 0,
  ]).sort())
  const listQuery = useMemo(() => emailQuery({
    folderId,
    folderRole,
    unread: filter === 'unread' ? true : null,
    text: query || null,
    limit: PAGE_SIZE,
  }), [folderId, folderRole, filter, query])
  const listKey = useMemo(() => mailKeys.emails(sourceIdentity, listQuery), [sourceIdentity, listQuery])

  const list = useInfiniteQuery({
    queryKey: listKey,
    queryFn: ({ pageParam }) => client.listEmails({ ...listQuery, cursor: pageParam }),
    initialPageParam: null as string | null,
    getNextPageParam: (page) => page.nextCursor,
    enabled: !treeLoading,
  })

  // Only data in this view's folders refreshes it; a refetch reloads every
  // page already shown, so scrolling position and "load more" progress survive.
  const listedRevision = useRef({ key: listKey, revision: scopeRevision })
  useEffect(() => {
    const listed = listedRevision.current
    listedRevision.current = { key: listKey, revision: scopeRevision }
    if (listed.key === listKey && listed.revision !== scopeRevision) {
      void queryClient.invalidateQueries({ queryKey: listKey, exact: true })
    }
  }, [queryClient, listKey, scopeRevision])

  const pages = list.data?.pages
  const emails = useMemo(
    () => [...new Map((pages ?? []).flatMap((page) => page.items).map((email) => [email.id, email])).values()],
    [pages],
  )
  // A selection lasts only while the email is in the current list, so a
  // refresh that drops it, a filter that hides it or a server change clears it.
  const selected = selectedId && emails.some((email) => email.id === selectedId) ? selectedId : null

  const detail = useQuery({
    queryKey: mailKeys.email(selected ?? ''),
    queryFn: () => client.getEmail(selected as string),
    enabled: selected !== null,
  })
  // List refreshes carry the latest flags; show them without refetching the detail.
  const listedSummary = emails.find((email) => email.id === selected)
  const detailData = detail.data && listedSummary ? { ...detail.data, summary: listedSummary } : (detail.data ?? null)

  const loadMore = () => {
    if (list.hasNextPage && !list.isFetching) void list.fetchNextPage()
  }

  const dialog = (
    <SettingsDialog
      client={client}
      target={settings}
      onClose={() => setSettings(null)}
      onAccountsChanged={() => void queryClient.invalidateQueries({ queryKey: mailKeys.all })}
    />
  )
  if (!treeLoading && !tree.isError && accounts.length === 0) {
    return (
      <>
        <NoAccounts onAddAccount={() => setSettings({ section: 'accounts', newAccount: true })} />
        {dialog}
      </>
    )
  }
  const showAccountBadges = activeMailbox.kind === 'unified' && accounts.length > 1
  const firstPageError = list.isError && !list.isFetchNextPageError ? clientFailure(list.error).message : null

  const selectMailbox = (next: Mailbox) => {
    setMailbox(next)
    setSelectedId(null)
  }

  return (
    <TooltipProvider>
      <SidebarProvider className="h-full min-h-0">
        <MailSidebar
          accounts={accounts}
          folders={folders}
          loading={treeLoading}
          syncStates={currentSyncStates}
          mailbox={activeMailbox}
          onSelect={selectMailbox}
          onOpenSettings={() => setSettings({ section: 'general' })}
        />
        <TitleBarActions side="leading">
          <SidebarTrigger aria-label="折叠或展开侧边栏" />
        </TitleBarActions>
        <SidebarInset className="min-h-0 min-w-0">
          <ResizablePanelGroup orientation="horizontal" className="min-h-0">
            <ResizablePanel defaultSize="40" minSize={300}>
              <EmailList
                title={mailboxTitle(activeMailbox)}
                emails={emails}
                accounts={showAccountBadges ? accounts : null}
                loading={treeLoading || list.isPending}
                syncStatus={folderSyncStatus(scopedFolders, currentSyncStates, pages?.at(-1)?.coverage.windowCapped ?? false)}
                hasMore={list.hasNextPage}
                loadingMore={list.isFetchingNextPage}
                refreshing={list.isFetching && !list.isFetchingNextPage}
                loadMoreError={list.isFetchNextPageError ? clientFailure(list.error).message : null}
                error={tree.isError ? clientFailure(tree.error).message : firstPageError}
                filter={filter}
                search={search}
                selectedId={selected}
                onFilterChange={setFilter}
                onSearchChange={setSearch}
                onSelect={(email) => setSelectedId(email.id)}
                onLoadMore={loadMore}
              />
            </ResizablePanel>
            <ResizableHandle withHandle />
            <ResizablePanel defaultSize="60" minSize={360}>
              <EmailView
                email={selected ? detailData : null}
                loading={selected !== null && detail.isPending}
                error={selected && detail.isError ? clientFailure(detail.error).message : null}
              />
            </ResizablePanel>
          </ResizablePanelGroup>
        </SidebarInset>
      </SidebarProvider>
      {dialog}
    </TooltipProvider>
  )
}

function NoAccounts({ onAddAccount }: { onAddAccount: () => void }) {
  return (
    <Empty className="h-full">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <HugeiconsIcon icon={Mail01Icon} />
        </EmptyMedia>
        <EmptyTitle>还没有邮箱账号</EmptyTitle>
        <EmptyDescription>添加一个邮箱后，邮件会显示在这里。</EmptyDescription>
      </EmptyHeader>
      <EmptyContent>
        <Button onClick={onAddAccount}>
          <HugeiconsIcon icon={Add01Icon} />
          添加邮箱账号
        </Button>
      </EmptyContent>
    </Empty>
  )
}
