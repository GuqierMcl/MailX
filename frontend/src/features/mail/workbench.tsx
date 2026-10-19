import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import { Add01Icon, Mail01Icon } from '@hugeicons/core-free-icons'
import type { Account, AccountSyncState, EmailDetail, EmailListQuery, EmailPage, EmailSummary, Folder } from '@/generated/accounts'
import { Button } from '@/components/ui/button'
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/components/ui/resizable'
import { SidebarInset, SidebarProvider, SidebarTrigger } from '@/components/ui/sidebar'
import { TooltipProvider } from '@/components/ui/tooltip'
import { clientFailure, type MailxClient } from '@/lib/mailx-client'
import { EmailList, type ListFilter } from './email-list'
import { EmailView } from './email-view'
import { emailQuery } from './format'
import { TitleBarActions } from '@/features/window-chrome/title-bar'
import { SettingsDialog, type SettingsTarget } from '@/features/settings/settings-dialog'
import { MailSidebar } from './mail-sidebar'
import { mailboxTitle, type Mailbox } from './mailbox'
import { acceptsSyncState, folderSyncStatus } from './folder-sync-state'

type Loadable<T> = { data: T; loading: boolean; error: string | null }
type EmailListState = Loadable<EmailSummary[]> & {
  query: EmailListQuery | null
  sourceIdentity: string
  coverage: EmailPage['coverage'] | null
  nextCursor: string | null
  refreshing: boolean
  loadingMore: boolean
  loadMoreError: string | null
}

const SEARCH_DELAY_MS = 200
const TREE_RELOAD_DELAY_MS = 250

export function Workbench({ client }: { client: MailxClient }) {
  const [settings, setSettings] = useState<SettingsTarget | null>(null)
  const [treeVersion, setTreeVersion] = useState(0)
  const [tree, setTree] = useState<Loadable<{ accounts: Account[]; folders: Folder[] }>>({
    data: { accounts: [], folders: [] },
    loading: true,
    error: null,
  })
  const [mailbox, setMailbox] = useState<Mailbox>({ kind: 'unified', role: 'inbox' })
  const [filter, setFilter] = useState<ListFilter>('all')
  const [search, setSearch] = useState('')
  const [query, setQuery] = useState('')
  const [list, setList] = useState<EmailListState>({
    data: [],
    loading: true,
    error: null,
    query: null,
    sourceIdentity: '',
    coverage: null,
    nextCursor: null,
    refreshing: false,
    loadingMore: false,
    loadMoreError: null,
  })
  const listRequest = useRef(0)
  const loadedPages = useRef<{ query: EmailListQuery | null; count: number }>({ query: null, count: 1 })
  const listSource = useRef('')
  const [syncStates, setSyncStates] = useState<Record<string, AccountSyncState>>({})
  const syncSnapshots = useRef<Record<string, AccountSyncState>>({})
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [detail, setDetail] = useState<Loadable<EmailDetail | null>>({ data: null, loading: false, error: null })

  useEffect(() => {
    let active = true
    ;(async () => {
      try {
        const accounts = await client.listAccounts()
        const folders = (await Promise.all(accounts.map((account) => client.listFolders(account.id)))).flat()
        if (active) setTree({ data: { accounts, folders }, loading: false, error: null })
      } catch (cause) {
        if (active) setTree((current) => ({ ...current, loading: false, error: clientFailure(cause).message }))
      }
    })()
    return () => {
      active = false
    }
  }, [client, treeVersion])

  const { accounts, folders } = tree.data
  // One sync round can change many folders; reload the tree once for the burst.
  // The timer outlives the subscription effect below: snapshots are already
  // updated when it is scheduled, so dropping it on re-subscribe would lose the reload.
  const treeReloadTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  const scheduleTreeReload = useCallback(() => {
    clearTimeout(treeReloadTimer.current)
    treeReloadTimer.current = setTimeout(() => setTreeVersion((version) => version + 1), TREE_RELOAD_DELAY_MS)
  }, [])
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
  }, [client, accountIdentity, scheduleTreeReload])

  useEffect(() => {
    const timer = setTimeout(() => setQuery(search.trim()), SEARCH_DELAY_MS)
    return () => clearTimeout(timer)
  }, [search])

  const currentFolder = mailbox.kind === 'folder' ? folders.find((folder) => folder.id === mailbox.folder.id && folder.selectable) : null
  const activeMailbox: Mailbox = mailbox.kind === 'folder' && !tree.loading
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
    limit: 100,
  }), [folderId, folderRole, filter, query])
  useEffect(() => {
    if (tree.loading) return
    const request = ++listRequest.current
    const sameSource = listSource.current === sourceIdentity
    if (!sameSource) {
      listSource.current = sourceIdentity
      setSelectedId(null)
      setDetail({ data: null, loading: false, error: null })
    }
    const pageCount = sameSource && loadedPages.current.query === listQuery ? loadedPages.current.count : 1
    loadedPages.current = { query: listQuery, count: pageCount }
    setList((current) => ({
      ...current,
      data: sameSource && current.query === listQuery ? current.data : [],
      loading: !sameSource || current.query !== listQuery,
      refreshing: true,
      loadingMore: false,
      error: null,
      loadMoreError: null,
      query: listQuery,
      sourceIdentity,
    }))
    const refresh = async () => {
      let page = await client.listEmails(listQuery)
      const items = [...page.items]
      let count = 1
      while (page.nextCursor && count < pageCount && request === listRequest.current) {
        page = await client.listEmails({ ...listQuery, cursor: page.nextCursor })
        items.push(...page.items)
        count += 1
      }
      if (request === listRequest.current) loadedPages.current = { query: listQuery, count }
      return { ...page, items: [...new Map(items.map((item) => [item.id, item])).values()] }
    }
    refresh().then(
      (page) => {
        if (request !== listRequest.current) return
        setList({
          data: page.items,
          loading: false,
          error: null,
          query: listQuery,
          sourceIdentity,
          coverage: page.coverage,
          nextCursor: page.nextCursor,
          refreshing: false,
          loadingMore: false,
          loadMoreError: null,
        })
        setSelectedId((current) => page.items.some((email) => email.id === current) ? current : null)
        setDetail((current) => {
          const summary = page.items.find((email) => email.id === current.data?.summary.id)
          return summary && current.data ? { ...current, data: { ...current.data, summary } } : current
        })
      },
      (cause) => {
        if (request !== listRequest.current) return
        setList((current) => ({ ...current, loading: false, refreshing: false, error: clientFailure(cause).message }))
      },
    )
    return () => {
      // Invalidate both the first page and any outstanding pagination from this view.
      listRequest.current += 1
    }
  }, [client, listQuery, scopeRevision, sourceIdentity, tree.loading])

  const loadMore = async () => {
    if (!list.nextCursor || list.refreshing || list.loadingMore) return
    const request = listRequest.current
    setList((current) => ({ ...current, loadingMore: true, loadMoreError: null }))
    try {
      const page = await client.listEmails({ ...listQuery, cursor: list.nextCursor })
      if (request !== listRequest.current) return
      loadedPages.current.count += 1
      setList((current) => {
        const ids = new Set(current.data.map((email) => email.id))
        return {
          ...current,
          data: [...current.data, ...page.items.filter((email) => !ids.has(email.id))],
          coverage: page.coverage,
          nextCursor: page.nextCursor,
          loadingMore: false,
        }
      })
    } catch (cause) {
      if (request !== listRequest.current) return
      setList((current) => ({ ...current, loadingMore: false, loadMoreError: clientFailure(cause).message }))
    }
  }

  useEffect(() => {
    if (!selectedId) return
    let active = true
    setDetail({ data: null, loading: true, error: null })
    client.getEmail(selectedId).then(
      (email) => active && setDetail({ data: email, loading: false, error: null }),
      (cause) => active && setDetail({ data: null, loading: false, error: clientFailure(cause).message }),
    )
    return () => {
      active = false
    }
  }, [client, selectedId])

  const dialog = (
    <SettingsDialog
      client={client}
      target={settings}
      onClose={() => setSettings(null)}
      onAccountsChanged={() => setTreeVersion((version) => version + 1)}
    />
  )
  if (!tree.loading && !tree.error && accounts.length === 0) {
    return (
      <>
        <NoAccounts onAddAccount={() => setSettings({ section: 'accounts', newAccount: true })} />
        {dialog}
      </>
    )
  }
  const showAccountBadges = activeMailbox.kind === 'unified' && accounts.length > 1
  const currentView = list.sourceIdentity === sourceIdentity && list.query === listQuery

  const selectMailbox = (next: Mailbox) => {
    setMailbox(next)
    setSelectedId(null)
    setDetail({ data: null, loading: false, error: null })
  }

  return (
    <TooltipProvider>
      <SidebarProvider className="h-full min-h-0">
        <MailSidebar
          accounts={accounts}
          folders={folders}
          loading={tree.loading}
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
                emails={currentView ? list.data : []}
                accounts={showAccountBadges ? accounts : null}
                loading={!currentView || list.loading}
                syncStatus={folderSyncStatus(scopedFolders, currentSyncStates, list.coverage?.windowCapped ?? false)}
                hasMore={list.nextCursor !== null}
                loadingMore={list.loadingMore}
                refreshing={list.refreshing}
                loadMoreError={list.loadMoreError}
                error={tree.error ?? list.error}
                filter={filter}
                search={search}
                selectedId={currentView ? selectedId : null}
                onFilterChange={setFilter}
                onSearchChange={setSearch}
                onSelect={(email) => setSelectedId(email.id)}
                onLoadMore={loadMore}
              />
            </ResizablePanel>
            <ResizableHandle withHandle />
            <ResizablePanel defaultSize="60" minSize={360}>
              <EmailView email={currentView && selectedId ? detail.data : null} loading={currentView && selectedId !== null && detail.loading} error={currentView && selectedId ? detail.error : null} />
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
