import { useEffect, useMemo, useRef, useState } from 'react'
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

type Loadable<T> = { data: T; loading: boolean; error: string | null }
type EmailListState = Loadable<EmailSummary[]> & {
  query: EmailListQuery | null
  coverage: EmailPage['coverage'] | null
  nextCursor: string | null
  refreshing: boolean
  loadingMore: boolean
  loadMoreError: string | null
}

const SEARCH_DELAY_MS = 200

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
    coverage: null,
    nextCursor: null,
    refreshing: false,
    loadingMore: false,
    loadMoreError: null,
  })
  const listRequest = useRef(0)
  const [syncStates, setSyncStates] = useState<Record<string, AccountSyncState>>({})
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

  useEffect(() => {
    let active = true
    const apply = (state: AccountSyncState) => setSyncStates((current) => ({ ...current, [state.accountId]: state }))
    // A finished sync means fresh folders, counts and headers; events are only hints to refetch.
    const stop = client.onSyncState((state) => {
      if (!active) return
      apply(state)
      if (state.phase === 'idle') setTreeVersion((version) => version + 1)
    })
    client.listSyncStates().then(
      (states) => active && states.forEach(apply),
      () => undefined,
    )
    return () => {
      active = false
      stop()
    }
  }, [client])

  useEffect(() => {
    const timer = setTimeout(() => setQuery(search.trim()), SEARCH_DELAY_MS)
    return () => clearTimeout(timer)
  }, [search])

  const folderId = mailbox.kind === 'folder' ? mailbox.folder.id : null
  const folderRole = mailbox.kind === 'unified' ? mailbox.role : null
  const listQuery = useMemo(() => emailQuery({
    folderId,
    folderRole,
    unread: filter === 'unread' ? true : null,
    text: query || null,
    limit: 100,
  }), [folderId, folderRole, filter, query])
  useEffect(() => {
    const request = ++listRequest.current
    setList((current) => ({
      ...current,
      data: current.query === listQuery ? current.data : [],
      loading: current.query !== listQuery,
      refreshing: true,
      loadingMore: false,
      error: null,
      loadMoreError: null,
      query: listQuery,
    }))
    client.listEmails(listQuery).then(
      (page) => {
        if (request !== listRequest.current) return
        setList({
          data: page.items,
          loading: false,
          error: null,
          query: listQuery,
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
  }, [client, listQuery, treeVersion])

  const loadMore = async () => {
    if (!list.nextCursor || list.refreshing || list.loadingMore) return
    const request = listRequest.current
    setList((current) => ({ ...current, loadingMore: true, loadMoreError: null }))
    try {
      const page = await client.listEmails({ ...listQuery, cursor: list.nextCursor })
      if (request !== listRequest.current) return
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

  const { accounts, folders } = tree.data
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
  const showAccountBadges = mailbox.kind === 'unified' && accounts.length > 1
  const isInbox = (mailbox.kind === 'unified' ? mailbox.role : mailbox.folder.role) === 'inbox'

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
          syncStates={syncStates}
          mailbox={mailbox}
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
                title={mailboxTitle(mailbox)}
                emails={list.data}
                accounts={showAccountBadges ? accounts : null}
                loading={list.loading}
                emptyState={!isInbox ? 'unsupported' : list.coverage?.headersComplete ? 'empty' : 'syncing'}
                hasMore={list.nextCursor !== null}
                loadingMore={list.loadingMore}
                refreshing={list.refreshing}
                loadMoreError={list.loadMoreError}
                error={tree.error ?? list.error}
                filter={filter}
                search={search}
                selectedId={selectedId}
                onFilterChange={setFilter}
                onSearchChange={setSearch}
                onSelect={(email) => setSelectedId(email.id)}
                onLoadMore={loadMore}
              />
            </ResizablePanel>
            <ResizableHandle withHandle />
            <ResizablePanel defaultSize="60" minSize={360}>
              <EmailView email={selectedId ? detail.data : null} loading={selectedId !== null && detail.loading} error={selectedId ? detail.error : null} />
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
