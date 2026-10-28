import { errorText } from '@/i18n/errors'
import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import { Add01Icon, Mail01Icon } from '@hugeicons/core-free-icons'
import { useInfiniteQuery, useQuery, useQueryClient } from '@tanstack/react-query'
import type { AccountSyncState } from '@/generated/accounts'
import { Button } from '@/components/ui/button'
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { usePanelRef } from 'react-resizable-panels'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/components/ui/resizable'
import { TooltipProvider } from '@/components/ui/tooltip'
import { clientFailureCode, type MailxClient } from '@/lib/mailx-client'
import { mailKeys } from '@/lib/query-client'
import { EmailList, type ListFilter } from './email-list'
import { CommandPalette } from './command-palette'
import { MailSearch } from './mail-search'
import { EmailView } from './email-view'
import { emailQuery } from './format'
import { NAV_DEFAULT } from '@/features/shell/nav-width'
import { clampListWidth, LIST_DEFAULT, LIST_MIN, READER_MIN } from '@/features/shell/layout-defaults'
import { TitleBarActions } from '@/features/window-chrome/title-bar'
import { ShellLayout } from '@/features/shell/shell-layout'
import { SettingsDialog, type SettingsTarget } from '@/features/settings/settings-dialog'
import { AccountsDialog, type AccountsTarget } from '@/features/accounts/accounts-dialog'
import { AddAccountDialog } from '@/features/accounts/add-account-dialog'
import { useDebouncedLayoutSave, useSettings } from '@/features/settings/settings-queries'
import { AvatarSourceProvider } from './sender-avatar'
import { MailSidebar } from './mail-sidebar'
import { ReaderToggle } from './reader-toggle'
import { folderLabel, mailboxTitle, type Mailbox } from './mailbox'
import { acceptsSyncState, folderSyncStatus } from './folder-sync-state'
import { useEmailBody, type OpenedEmail } from './use-email-body'
import { useMarkOpenedAsRead, useMessageFlags } from './use-message-flags'
import { loadMailTree } from './mail-tree'
import { isMailView } from './mailbox'
import { DraftList, OutboxList } from '@/features/outbox/compose-lists'
import { DraftDetail, DraftPlaceholder, OutboxDetail } from '@/features/outbox/compose-details'
import { useComposeActions, useComposeEvents, useDrafts, useOutbox } from '@/features/outbox/use-compose'
import { clearRemovedAccount } from './account-removal'

const SEARCH_DELAY_MS = 200
const TREE_RELOAD_DELAY_MS = 250
const PAGE_SIZE = 100

/** What was opened, and which server of its account it came from at that moment. */
type OpenedState = OpenedEmail & { source: string | null }

export function Workbench({ client }: { client: MailxClient }) {
  useTranslation()
  const queryClient = useQueryClient()
  const [settings, setSettings] = useState<SettingsTarget | null>(null)
  const listPanel = usePanelRef()
  const readerPanel = usePanelRef()
  // Whether the reader is folded away: what was saved, until the user changes it.
  // The state is the truth and the panel follows it; dragging the handle past the
  // reader's minimum width collapses the panel by itself, and `onResize` below
  // reports that back here. Every change is saved (debounced) further down.
  const [readerChoice, setReaderChoice] = useState<boolean | null>(null)
  const saveLayout = useDebouncedLayoutSave(client)
  const [accountsTarget, setAccountsTarget] = useState<AccountsTarget | null>(null)
  const [addingAccount, setAddingAccount] = useState(false)
  const [mailbox, setMailbox] = useState<Mailbox>({ kind: 'unified', role: 'inbox' })
  const [filter, setFilter] = useState<ListFilter>('all')
  const [search, setSearch] = useState('')
  const [query, setQuery] = useState('')
  const [paletteOpen, setPaletteOpen] = useState(false)
  const [syncStates, setSyncStates] = useState<Record<string, AccountSyncState>>({})
  const syncSnapshots = useRef<Record<string, AccountSyncState>>({})
  const removedAccounts = useRef(new Set<string>())
  // The message the reader shows. It is its own state, not "whichever listed message
  // matches": changing mailbox, filter or search, or the list refreshing, never
  // closes it. Only opening another message, closing it, or it ceasing to exist does.
  const [openedState, setOpened] = useState<OpenedState | null>(null)
  // The draft or outbox item shown in the reader while those views are open.
  const [draftChoice, setDraftChoice] = useState<string | null>(null)
  const [outboxChoice, setOutboxChoice] = useState<string | null>(null)

  const tree = useQuery({
    queryKey: mailKeys.tree(),
    queryFn: async () => {
      const tree = await loadMailTree(client)
      return {
        accounts: tree.accounts.filter((account) => !removedAccounts.current.has(account.id)),
        folders: tree.folders.filter((folder) => !removedAccounts.current.has(folder.accountId)),
      }
    },
  })
  const settingsQuery = useSettings(client)
  const senderAvatars = settingsQuery.data?.settings.security.senderAvatars ?? false
  const navWidth = settingsQuery.data?.settings.layout.navWidth ?? NAV_DEFAULT
  const previewLines = settingsQuery.data?.settings.appearance.previewLines ?? 'one'
  const listWidth = clampListWidth(settingsQuery.data?.settings.layout.listWidth ?? LIST_DEFAULT)
  const layoutReady = !settingsQuery.isPending
  const savedCollapsed = settingsQuery.data?.settings.layout.readerCollapsed ?? false
  const readerCollapsed = readerChoice ?? savedCollapsed
  const setReaderCollapsed = setReaderChoice // stable, so effects may use it
  useLayoutEffect(() => {
    const reader = readerPanel.current
    if (!layoutReady || !reader) return
    if (readerCollapsed && !reader.isCollapsed()) reader.collapse()
    else if (!readerCollapsed && reader.isCollapsed()) reader.expand()
  }, [layoutReady, readerCollapsed, readerPanel])
  // Each change is handed over as it happens; the debounce keeps only the last. Comparing
  // with the saved value here would let an earlier queued value win after a quick toggle back.
  useEffect(() => {
    if (readerChoice !== null) saveLayout({ readerCollapsed: readerChoice })
  }, [readerChoice, saveLayout])
  const treeLoading = tree.isPending
  const accounts = useMemo(() => tree.data?.accounts ?? [], [tree.data])
  const folders = useMemo(() => tree.data?.folders ?? [], [tree.data])

  const drafts = useDrafts(client)
  const outbox = useOutbox(client)
  const draftCount = drafts.data?.length ?? 0
  const outboxCount = outbox.data?.length ?? 0
  // The outbox entry exists only while something is unfinished; if the view is open
  // when the last item completes, fall back to the inbox.
  const outboxGone = mailbox.kind === 'outbox' && outbox.isSuccess && outboxCount === 0
  const selectOutboxItem = useCallback((outboxId: string) => {
    setOutboxChoice(outboxId)
    setMailbox({ kind: 'outbox' })
  }, [])
  useComposeEvents(client, selectOutboxItem)

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

  useEffect(() => client.onAccountRemoved((report) => {
    removedAccounts.current.add(report.accountId)
    clearTimeout(treeReloadTimer.current)
    const nextSync = { ...syncSnapshots.current }
    delete nextSync[report.accountId]
    syncSnapshots.current = nextSync
    setSyncStates(nextSync)
    setOpened((opened) => opened?.accountId === report.accountId ? null : opened)
    setMailbox((current) => current.kind === 'folder' && current.folder.accountId === report.accountId
      ? { kind: 'unified', role: 'inbox' } : current)
    setDraftChoice((id) => id && report.draftIds.includes(id) ? null : id)
    clearRemovedAccount(queryClient, report)
  }), [client, queryClient])

  const accountIdentity = JSON.stringify(accounts.map(({ id, mailboxSourceId }) => [id, mailboxSourceId]).sort())
  useEffect(() => {
    let active = true
    const currentAccounts = new Map<string, string>(JSON.parse(accountIdentity))
    const apply = (state: AccountSyncState) => {
      if (!active || removedAccounts.current.has(state.accountId)) return
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
  const activeMailbox: Mailbox = outboxGone
    ? { kind: 'unified', role: 'inbox' }
    : mailbox.kind === 'folder' && !treeLoading
      ? currentFolder ? { kind: 'folder', folder: currentFolder } : { kind: 'unified', role: 'inbox' }
      : mailbox
  const mailView = isMailView(activeMailbox)
  const folderId = activeMailbox.kind === 'folder' ? activeMailbox.folder.id : null
  const folderRole = activeMailbox.kind === 'unified' ? activeMailbox.role : null
  const selectedAccountId = activeMailbox.kind === 'folder' ? activeMailbox.folder.accountId : null
  const compose = useComposeActions(client, accounts, selectedAccountId)
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
    enabled: !treeLoading && mailView,
    // Keep the previous rows (dimmed) while the next folder or filter loads, so
    // switching never blanks the list. Rows from another mailbox source (a
    // changed server) are never carried over.
    placeholderData: (previous, previousQuery) =>
      previousQuery?.queryKey[2] === sourceIdentity ? previous : undefined,
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
  const body = useEmailBody(client, openedState, accounts, currentSyncStates)
  // What must close the reader: the message is gone for good (deleted, or moved out
  // of sight by the server), or its account was replaced. Neither is a list change.
  // Being closed is derived, not stored, so there is nothing to keep in step.
  const accountsKnown = !treeLoading && !tree.isError
  const source = (accountId: string) => accounts.find((account) => account.id === accountId)?.mailboxSourceId ?? null
  const closedByAccount = Boolean(
    openedState && accountsKnown && source(openedState.accountId) !== openedState.source,
  )
  const closedByLoss = Boolean(
    openedState && body.detail.isError && clientFailureCode(body.detail.error) === 'notFound',
  )
  const opened = openedState && !closedByAccount && !closedByLoss ? openedState : null
  // The list only highlights the opened message when it happens to be listed.
  const highlighted = opened && emails.some((email) => email.id === opened.id) ? opened.id : null
  const { setFlag } = useMessageFlags(client)
  // The mail is "opened" once its detail is on screen; a mail the user only
  // moved past stays unread.
  useMarkOpenedAsRead(
    opened ? body.email?.summary : null,
    Boolean(opened && body.email && !body.detail.isPending),
    (email, flag, wanted) => void setFlag(email, flag, wanted),
  )

  // A clicked notification: one message opens in the reader (wherever the list
  // is), several show that account's inbox. Reads the latest tree through a ref
  // so the subscription does not restart on every tree change.
  const latestTree = useRef({ accounts, folders })
  useEffect(() => { latestTree.current = { accounts, folders } }, [accounts, folders])
  useEffect(() => client.onNotificationOpen(({ accountId, emailId, outboxId }) => {
    if (outboxId) {
      setOutboxChoice(outboxId)
      setMailbox({ kind: 'outbox' })
      return
    }
    const { accounts, folders } = latestTree.current
    const account = accounts.find((row) => row.id === accountId)
    if (!account) return
    if (emailId) {
      setOpened({ id: emailId, accountId, source: account.mailboxSourceId })
      setReaderChoice(false)
      return
    }
    const inbox = folders.find((folder) => folder.accountId === accountId && folder.role === 'inbox' && folder.selectable)
    if (inbox) {
      setSearch('')
      setFilter('all')
      setMailbox({ kind: 'folder', folder: inbox })
    }
  }), [client])

  // Cmd/Ctrl+N writes a new mail from anywhere in the main window.
  const composeNew = compose.compose
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key.toLowerCase() === 'n' && (event.metaKey || event.ctrlKey) && !event.altKey && !event.shiftKey) {
        event.preventDefault()
        void composeNew()
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [composeNew])

  const loadMore = () => {
    if (list.hasNextPage && !list.isFetching) void list.fetchNextPage()
  }

  const refreshAccounts = () => void queryClient.invalidateQueries({ queryKey: mailKeys.all })
  const dialog = (
    <>
      <SettingsDialog client={client} target={settings} onClose={() => setSettings(null)} />
      <AccountsDialog
        client={client}
        target={accountsTarget}
        accounts={accounts}
        syncStates={currentSyncStates}
        onClose={() => setAccountsTarget(null)}
        onAccountsChanged={refreshAccounts}
        onAddAccount={() => setAddingAccount(true)}
      />
      <AddAccountDialog
        client={client}
        open={addingAccount}
        accounts={accounts}
        onClose={() => setAddingAccount(false)}
        onCreated={(account) => {
          refreshAccounts()
          setAddingAccount(false)
          // Continue in the account manager with the new account selected.
          setAccountsTarget({ accountId: account.id })
        }}
      />
    </>
  )
  if (!treeLoading && !tree.isError && accounts.length === 0) {
    return (
      <>
        <NoAccounts onAddAccount={() => setAddingAccount(true)} />
        {dialog}
      </>
    )
  }
  const showAccountBadges = activeMailbox.kind === 'unified' && accounts.length > 1
  // Drafts and outbox are not searched; the title-bar search then speaks for the inbox.
  const searchScope = !mailView ? mailboxTitle({ kind: 'unified', role: 'inbox' }) : activeMailbox.kind === 'folder'
    ? `${accounts.find((account) => account.id === activeMailbox.folder.accountId)?.displayName ?? ''} · ${folderLabel(activeMailbox.folder)}`
    : mailboxTitle(activeMailbox)
  const firstPageError = list.isError && !list.isFetchNextPageError ? errorText(list.error) : null

  const selectMailbox = (next: Mailbox) => setMailbox(next)
  // A search or a found message belongs to a mail list: leave the local views for it.
  const leaveLocalView = () => { if (!mailView) setMailbox({ kind: 'unified', role: 'inbox' }) }
  const selectedDraft = drafts.data?.some((draft) => draft.id === draftChoice) ? draftChoice : null
  const selectedOutbox = outbox.data?.find((item) => item.id === outboxChoice) ?? null

  // Every way of opening a message (list click, search result, notification) shows the reader.
  const openEmail = (email: { id: string; accountId: string }) => {
    setOpened({ id: email.id, accountId: email.accountId, source: source(email.accountId) })
    setReaderCollapsed(false)
  }
  const toggleReader = () => setReaderCollapsed(!readerCollapsed)
  // Only the user's own drags are saved, and not while the reader is folded away
  // (the list then fills the window, which is not the width to come back to).
  const saveListWidth = (_layout: unknown, meta: { isUserInteraction?: boolean } | undefined) => {
    if (!meta?.isUserInteraction || readerPanel.current?.isCollapsed()) return
    const width = listPanel.current?.getSize().inPixels
    if (width === undefined) return
    const next = clampListWidth(width)
    if (next !== listWidth) saveLayout({ listWidth: next })
  }

  return (
    <TooltipProvider>
      <AvatarSourceProvider client={client} enabled={senderAvatars}>
      <ShellLayout
        navLabel={t('mail:mailboxNavigation')}
        navWidth={navWidth}
        onNavWidthChange={(width) => saveLayout({ navWidth: width })}
        nav={(
          <MailSidebar
            accounts={accounts}
            folders={folders}
            loading={treeLoading}
            syncStates={currentSyncStates}
            mailbox={activeMailbox}
            onSelect={selectMailbox}
            onOpenAccounts={() => setAccountsTarget({})}
            onOpenSettings={() => setSettings({ section: 'general' })}
            onCompose={() => void compose.compose()}
            draftCount={draftCount}
            outboxCount={outboxCount}
            onRefresh={(accountId) => void client.refreshSync(accountId).catch(() => undefined)}
          />
        )}
      >
        <TitleBarActions side="center">
          <MailSearch value={mailView ? search : ''} scope={searchScope} onOpen={() => setPaletteOpen(true)} onClear={() => setSearch('')} />
        </TitleBarActions>
        <CommandPalette
          client={client}
          open={paletteOpen}
          onOpenChange={setPaletteOpen}
          scope={searchScope}
          search={mailView ? search : ''}
          folderId={folderId}
          folderRole={mailView ? folderRole : 'inbox'}
          sourceIdentity={sourceIdentity}
          accounts={accounts}
          folders={folders}
          onApplySearch={(text) => {
            leaveLocalView()
            setSearch(text)
          }}
          onOpenEmail={(email, text) => {
            leaveLocalView()
            setSearch(text)
            openEmail(email)
          }}
          onCompose={() => void compose.compose()}
          onSelectMailbox={selectMailbox}
          onOpenAccounts={() => setAccountsTarget({})}
          onAddAccount={() => setAddingAccount(true)}
          onOpenSettings={setSettings}
        />
        {/* The saved list width is only read when the group mounts, so it waits for the settings. */}
        {!layoutReady ? <div className="min-h-0 flex-1 bg-muted/40" /> : (
        <ResizablePanelGroup orientation="horizontal" className="min-h-0" onLayoutChanged={saveListWidth}>
          <ResizablePanel panelRef={listPanel} defaultSize={listWidth} minSize={LIST_MIN} className="bg-muted/40">
            {activeMailbox.kind === 'drafts' ? (
              <DraftList
                drafts={drafts.data}
                loading={drafts.isPending}
                error={drafts.isError ? errorText(drafts.error) : null}
                accounts={accounts.length > 1 ? accounts : null}
                selectedId={selectedDraft}
                onSelect={(id) => { setDraftChoice(id); setReaderCollapsed(false) }}
              />
            ) : activeMailbox.kind === 'outbox' ? (
              <OutboxList
                items={outbox.data}
                loading={outbox.isPending}
                error={outbox.isError ? errorText(outbox.error) : null}
                accounts={accounts.length > 1 ? accounts : null}
                selectedId={selectedOutbox?.id ?? null}
                onSelect={(id) => { setOutboxChoice(id); setReaderCollapsed(false) }}
              />
            ) : (
            <EmailList
              title={mailboxTitle(activeMailbox)}
              emails={emails}
              accounts={showAccountBadges ? accounts : null}
              loading={treeLoading || list.isPending}
              syncStatus={folderSyncStatus(scopedFolders, currentSyncStates, pages?.at(-1)?.coverage.windowCapped ?? false)}
              hasMore={list.hasNextPage}
              loadingMore={list.isFetchingNextPage}
              refreshing={list.isFetching && !list.isFetchingNextPage}
              stale={list.isPlaceholderData}
              loadMoreError={list.isFetchNextPageError ? errorText(list.error) : null}
              error={tree.isError ? errorText(tree.error) : firstPageError}
              filter={filter}
              search={search}
              selectedId={highlighted}
              previewLines={previewLines}
              onFilterChange={setFilter}
              onSelect={openEmail}
              onLoadMore={loadMore}
            />
            )}
          </ResizablePanel>
          <ResizableHandle disabled={readerCollapsed} className={readerCollapsed ? 'pointer-events-none opacity-0' : undefined} />
          <ResizablePanel
            panelRef={readerPanel}
            minSize={READER_MIN}
            collapsible
            collapsedSize={0}
            onResize={() => {
              const collapsed = readerPanel.current?.isCollapsed() ?? false
              if (collapsed !== readerCollapsed) setReaderCollapsed(collapsed)
            }}
            // Folded away, the reader is out of reach for Tab and screen readers too.
            inert={readerCollapsed}
          >
            {activeMailbox.kind === 'drafts' ? (
              selectedDraft ? (
                <DraftDetail
                  key={selectedDraft}
                  client={client}
                  draftId={selectedDraft}
                  accounts={accounts}
                  onEdit={(id) => void compose.openDraft(id)}
                  onGone={() => setDraftChoice(null)}
                />
              ) : <DraftPlaceholder />
            ) : activeMailbox.kind === 'outbox' ? (
              <OutboxDetail
                key={selectedOutbox?.id ?? ''}
                client={client}
                item={selectedOutbox}
                accounts={accounts}
                onEdit={(id) => void compose.openDraft(id)}
              />
            ) : (
            <EmailView
              hidden={readerCollapsed}
              key={`${opened?.id ?? ''}:${body.identity}`}
              client={client}
              onFlag={(email, flag, wanted) => void setFlag(email, flag, wanted)}
              onClose={() => setOpened(null)}
              email={opened ? body.email : null}
              providerId={accounts.find((account) => account.id === opened?.accountId)?.providerId ?? null}
              loading={opened !== null && body.detail.isPending}
              error={opened && body.detail.isError ? errorText(body.detail.error) : null}
              downloading={body.pending}
              onDownload={body.download}
              onRespond={(kind) => opened && void compose.respond(opened.id, kind)}
            />
            )}
          </ResizablePanel>
        </ResizablePanelGroup>
        )}
        {accounts.length > 0 && <ReaderToggle collapsed={readerCollapsed} onToggle={toggleReader} />}
      </ShellLayout>
      </AvatarSourceProvider>
      {dialog}
    </TooltipProvider>
  )
}

function NoAccounts({ onAddAccount }: { onAddAccount: () => void }) {
  useTranslation()
  return (
    <Empty className="h-full">
      <EmptyHeader>
        <EmptyMedia variant="icon">
          <HugeiconsIcon icon={Mail01Icon} />
        </EmptyMedia>
        <EmptyTitle>{t('mail:noAccounts')}</EmptyTitle>
        <EmptyDescription>{t('mail:addAccountHint')}</EmptyDescription>
      </EmptyHeader>
      <EmptyContent>
        <Button onClick={onAddAccount}>
          <HugeiconsIcon icon={Add01Icon} />

          {t('mail:addAccount')}
        </Button>
      </EmptyContent>
    </Empty>
  )
}
