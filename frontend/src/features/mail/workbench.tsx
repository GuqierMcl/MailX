import { errorText } from '@/i18n/errors'
import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import { Add01Icon, Mail01Icon } from '@hugeicons/core-free-icons'
import { useInfiniteQuery, useQuery, useQueryClient } from '@tanstack/react-query'
import type { AccountSyncState, EmailListQuery, EmailSummary, FolderRole } from '@/generated/accounts'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { toast } from '@/components/ui/toast'
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import { usePanelRef } from 'react-resizable-panels'
import { ResizableHandle, ResizablePanel, ResizablePanelGroup } from '@/components/ui/resizable'
import { TooltipProvider } from '@/components/ui/tooltip'
import { clientFailureCode, type MailxClient } from '@/lib/mailx-client'
import { mailKeys } from '@/lib/query-client'
import { usePrefersReducedMotion } from '@/hooks/use-reduced-motion'
import { EmailList, type ListFilter, type RevealRequest } from './email-list'
import { escapeBelongsToSomethingElse } from './use-close-on-escape'
import { CommandPalette } from './command-palette'
import { MailSearch } from './mail-search'
import { ConversationFrame, type ThreadMode } from './conversation-view'
import { EmailView, RemoveLocalCopyDialog } from './email-view'
import { TranslationConsentDialog } from './translation/translation-consent-dialog'
import { useEmailTranslation } from './translation/use-email-translation'
import { emailQuery } from './format'
import { NAV_DEFAULT } from '@/features/shell/nav-width'
import { clampListWidth, LIST_DEFAULT, LIST_MIN, READER_MIN } from '@/features/shell/layout-defaults'
import { TitleBarActions } from '@/features/window-chrome/title-bar'
import { ShellLayout } from '@/features/shell/shell-layout'
import { useSettingsRequests } from '@/features/settings/settings-requests'
import { SettingsDialog, type SettingsTarget } from '@/features/settings/settings-dialog'
import { AccountsDialog, type AccountsTarget } from '@/features/accounts/accounts-dialog'
import { AddAccountDialog } from '@/features/accounts/add-account-dialog'
import { sortAccountsForDisplay } from '@/features/accounts/account-order'
import { useDebouncedLayoutSave, useSettings } from '@/features/settings/settings-queries'
import { AvatarSourceProvider } from './sender-avatar'
import { MailRail } from './mail-rail'
import { MailSidebar } from './mail-sidebar'
import { ReaderToggle } from './reader-toggle'
import { RefreshAllButton } from './refresh-all-button'
import { folderLabel, mailboxTitle, STARRED_EXCLUDED_ROLES, type Mailbox } from './mailbox'
import { acceptsSyncState, folderSyncStatus } from './folder-sync-state'
import { useEmailBody, type OpenedEmail } from './use-email-body'
import { useMarkOpenedAsRead, useMessageFlags } from './use-message-flags'
import { useEmailPreviews } from './use-email-previews'
import { loadMailTree } from './mail-tree'
import { DeleteForGoodDialog } from './delete-for-good-dialog'
import { canArchive, useArchiveEmail, useArchiveEmails } from './use-archive-email'
import { useListSelection } from './use-list-selection'
import { selectionAvailability, useSelectionActions } from './use-selection-actions'
import { SelectionSummary } from './selection-summary'
import { ShortcutHelpDialog } from './shortcut-help'
import type { SingleKeyShortcutId } from './shortcuts'
import { useMailShortcuts } from './use-mail-shortcuts'
import { useDeleteEmail } from './use-delete-email'
import { useContactAggregation } from './use-contact-aggregation'
import { useMailStateChanges } from './use-mail-state-changes'
import { isMailView } from './mailbox'
import { DraftList, OutboxList } from '@/features/outbox/compose-lists'
import { DraftDetail, DraftPlaceholder, OutboxDetail } from '@/features/outbox/compose-details'
import { useComposeActions, useComposeEvents, useDrafts, useOutbox } from '@/features/outbox/use-compose'
import { clearRemovedAccount } from './account-removal'
import { useDesktopActions } from '@/features/lifecycle/use-desktop-actions'

const SEARCH_DELAY_MS = 200
const TREE_RELOAD_DELAY_MS = 250
const PAGE_SIZE = 100
/** The core's limit on kept ids (`MAX_KEEP_IDS`); the oldest drop out first. */
const MAX_KEPT = 500
const NO_IDS: string[] = []
/** Mail in these folders never shows up under a contact. */
const CONTACT_EXCLUDED_ROLES: FolderRole[] = ['junk', 'trash']

/** What was opened, and which server of its account it came from at that moment. */
type OpenedState = OpenedEmail & { source: string | null }

/**
 * One sender's mail, drilled into from a list. It sits on top of the mailbox
 * that was showing (which stays highlighted); `accountId` is that mailbox's
 * account when it was scoped to one, so the contact view stays inside it.
 */
type ContactView = { address: string; name: string | null; accountId: string | null }

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
  // Messages opened in one Unread view; see `keptIds` below.
  const [kept, setKept] = useState<{ view: string; ids: string[] }>({ view: '', ids: NO_IDS })
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
  // The message whose conversation the reader shows. Every normal way of opening a
  // message sets it; expanding another message of the thread does not, so the thread
  // stays put while the expanded message changes.
  const [anchorId, setAnchorId] = useState<string | null>(null)
  const [threadMode, setThreadMode] = useState<ThreadMode>('thread')
  const [contact, setContact] = useState<ContactView | null>(null)
  // Which way the list slides on a base <-> contact switch; null for any other change.
  const [slide, setSlide] = useState<'forward' | 'back' | null>(null)
  // Scroll offset of the base list: followed live, copied when the contact view opens,
  // and handed back to the list that replaces the contact view.
  const liveScroll = useRef(0)
  const savedScroll = useRef(0)
  const [restoreScroll, setRestoreScroll] = useState(0)
  const trackScroll = useCallback((offset: number) => { liveScroll.current = offset }, [])
  const reducedMotion = usePrefersReducedMotion()
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
  const navCollapsed = settingsQuery.data?.settings.layout.navCollapsed ?? false
  const previewLines = settingsQuery.data?.settings.appearance.previewLines ?? 'one'
  const mailboxLabel = settingsQuery.data?.settings.appearance.mailboxLabel ?? 'name'
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
  const accountOrder = settingsQuery.data?.settings.layout.accountOrder
  const displayAccounts = useMemo(() => sortAccountsForDisplay(accounts, accountOrder ?? []), [accounts, accountOrder])
  const folders = useMemo(() => tree.data?.folders ?? [], [tree.data])

  const drafts = useDrafts(client)
  const outbox = useOutbox(client)
  const draftCount = drafts.data?.length ?? 0
  const outboxCount = outbox.data?.length ?? 0
  // The outbox entry exists only while something is unfinished; if the view is open
  // when the last item completes, fall back to the inbox.
  // Picking a mailbox from anywhere (sidebar, rail, palette, notification) ends the contact view.
  const showMailbox = useCallback((next: Mailbox) => {
    setContact(null)
    setSlide(null)
    setRestoreScroll(0)
    setMailbox(next)
  }, [])
  const outboxGone = mailbox.kind === 'outbox' && outbox.isSuccess && outboxCount === 0
  const selectOutboxItem = useCallback((outboxId: string) => {
    setOutboxChoice(outboxId)
    showMailbox({ kind: 'outbox' })
  }, [showMailbox])
  // An outbox change can add, drop or swap a local sent copy (a matched server
  // copy replaces it), so lists that include Sent scopes are refetched. If the
  // opened message was such a copy, its detail refetch reports notFound and the
  // reader closes through the usual "message ceased to exist" path.
  const refetchSentLists = useCallback(() => {
    void queryClient.invalidateQueries({ queryKey: mailKeys.emailLists() })
    void queryClient.invalidateQueries({ queryKey: [...mailKeys.all, 'email'] })
  }, [queryClient])
  useComposeEvents(client, selectOutboxItem, { onChange: refetchSentLists })
  // A copy removed from this device: the same refetch, without a send result.
  useEffect(() => client.onSentCopiesChanged(refetchSentLists), [client, refetchSentLists])

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
    setContact((current) => current?.accountId === report.accountId ? null : current)
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
      : mailbox.kind === 'archived' && !treeLoading && mailbox.accountId && !accounts.some((account) => account.id === mailbox.accountId)
        ? { kind: 'unified', role: 'inbox' }
        : mailbox
  const mailView = isMailView(activeMailbox)
  const folderId = activeMailbox.kind === 'folder' ? activeMailbox.folder.id : null
  const archivedView = activeMailbox.kind === 'archived'
  const starredView = activeMailbox.kind === 'starred'
  const folderRole = activeMailbox.kind === 'unified' ? activeMailbox.role : archivedView ? 'inbox' : null
  const selectedAccountId = activeMailbox.kind === 'folder' ? activeMailbox.folder.accountId : activeMailbox.kind === 'archived' ? activeMailbox.accountId : null
  const compose = useComposeActions(client, accounts, selectedAccountId)
  const sourceIdentity = JSON.stringify(accounts.filter((account) => !selectedAccountId || account.id === selectedAccountId)
    .map((account) => [account.id, account.mailboxSourceId]).sort())
  const currentSyncStates = Object.fromEntries(accounts.flatMap((account) => {
    const state = syncStates[account.id]
    return state?.mailboxSourceId === account.mailboxSourceId ? [[account.id, state]] : []
  }))
  const sentView = mailView && (folderRole === 'sent' || (activeMailbox.kind === 'folder' && activeMailbox.folder.role === 'sent'))
  const contactView = mailView ? contact : null
  const inboxView = mailView && !archivedView && !starredView && !contactView
    && (activeMailbox.kind === 'unified' ? activeMailbox.role === 'inbox' : activeMailbox.kind === 'folder' && activeMailbox.folder.role === 'inbox')
  const scopedFolders = folders.filter((folder) => folder.selectable && (contactView
    ? !CONTACT_EXCLUDED_ROLES.includes(folder.role) && (!contactView.accountId || folder.accountId === contactView.accountId)
    : folderId ? folder.id === folderId
      : starredView ? !STARRED_EXCLUDED_ROLES.includes(folder.role)
        : folder.role === folderRole && (!archivedView || !selectedAccountId || folder.accountId === selectedAccountId)))
  const scopeRevision = JSON.stringify(scopedFolders.map((folder) => [
    folder.id,
    accounts.find((account) => account.id === folder.accountId)?.mailboxSourceId,
    currentSyncStates[folder.accountId]?.generation ?? 0,
    currentSyncStates[folder.accountId]?.folders.find((state) => state.folderId === folder.id)?.dataRevision ?? 0,
  ]).sort())
  const listQuery = useMemo(() => contactView ? emailQuery({
    fromAddress: contactView.address,
    accountIds: contactView.accountId ? [contactView.accountId] : null,
    archive: 'include',
    excludeFolderRoles: CONTACT_EXCLUDED_ROLES,
    unread: filter === 'unread' ? true : null,
    text: query || null,
    limit: PAGE_SIZE,
  }) : emailQuery({
    folderId,
    folderRole,
    accountIds: archivedView && selectedAccountId ? [selectedAccountId] : null,
    archive: archivedView ? 'only' : starredView ? 'include' : null,
    starred: starredView ? true : null,
    excludeFolderRoles: starredView ? STARRED_EXCLUDED_ROLES : [],
    unread: filter === 'unread' ? true : null,
    text: query || null,
    // Grouped senders collapse into one row, in inbox views only.
    groupSenders: inboxView,
    limit: PAGE_SIZE,
  }), [contactView, folderId, folderRole, archivedView, starredView, inboxView, selectedAccountId, filter, query])
  const listKey = useMemo(() => mailKeys.emails(sourceIdentity, listQuery), [sourceIdentity, listQuery])
  // Mail opened from the Unread view stays listed, now read, until the view
  // changes (mailbox, filter or search), so rows do not vanish under the reader.
  // The ids ride along with every fetch of this list but are not part of its
  // key: opening a message must not start the list over from its first page.
  const viewKey = JSON.stringify(listKey)
  // Coming back to the same view later starts with nothing kept.
  if (kept.ids.length > 0 && kept.view !== viewKey) setKept({ view: '', ids: NO_IDS })
  const keptIds = kept.view === viewKey ? kept.ids : NO_IDS
  const keptRef = useRef(keptIds)
  useEffect(() => { keptRef.current = keptIds }, [keptIds])
  const hasKept = keptIds.length > 0
  useEffect(() => {
    if (!hasKept) return
    // Its cached pages hold the kept rows; the next visit starts without them.
    return () => queryClient.removeQueries({ queryKey: listKey, exact: true })
  }, [hasKept, listKey, queryClient])
  const selectQuery = useMemo(() => (keptIds.length > 0 ? { ...listQuery, keepIds: keptIds } : listQuery), [listQuery, keptIds])

  const list = useInfiniteQuery({
    queryKey: listKey,
    queryFn: ({ pageParam }) => client.listEmails({ ...listQuery, keepIds: keptRef.current, cursor: pageParam }),
    initialPageParam: null as string | null,
    getNextPageParam: (page) => page.nextCursor,
    enabled: !treeLoading && mailView,
    // Keep the previous rows (dimmed) while the next folder or filter loads, so
    // switching never blanks the list. Rows from another mailbox source (a
    // changed server) are never carried over, nor are those of another sender
    // (going into or out of a contact view starts from an empty list).
    placeholderData: (previous, previousQuery) =>
      previousQuery?.queryKey[2] === sourceIdentity &&
      (previousQuery.queryKey[3] as EmailListQuery).fromAddress === listQuery.fromAddress ? previous : undefined,
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
  // Picking messages: any change of mailbox, search or filter starts over.
  const selection = useListSelection(emails, JSON.stringify([activeMailbox.kind, listKey]))
  const selecting = mailView && selection.active
  const exitSelection = selection.exit
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
  // Translation of the opened message; the toolbar, notice bar, body, consent dialog and palette share it.
  const translation = useEmailTranslation(client, opened ? body.email : null, () => setSettings({ section: 'translation' }))
  // The list only highlights the opened message when it happens to be listed.
  const highlighted = opened && emails.some((email) => email.id === opened.id) ? opened.id : null
  const conversation = useQuery({
    queryKey: mailKeys.conversation(anchorId ?? ''),
    queryFn: () => client.getConversation(anchorId!),
    enabled: Boolean(anchorId && opened),
  })
  const threadPage = opened && conversation.data && anchorId ? conversation.data : null
  const threadPosition = threadPage?.messages.find((message) => message.id === opened?.id)
  // One-message threads, local sent copies, and loading or failed lookups keep today's reader.
  const threaded = Boolean(threadPage && threadPage.messages.length >= 2 && threadPosition && !threadPosition.localOnly)
  const { setFlag, setFlags } = useMessageFlags(client)
  useMailStateChanges(client)
  useEmailPreviews(client)
  const requestPreviews = useCallback((ids: string[]) => { void client.requestPreviews(ids).catch(() => undefined) }, [client])
  // The mail is "opened" once its detail is on screen; a mail the user only
  // moved past stays unread.
  useMarkOpenedAsRead(
    opened ? body.email?.summary : null,
    // The reader shows the selection summary instead while picking messages.
    Boolean(opened && body.email && !body.detail.isPending && !selecting),
    (email, flag, wanted) => void setFlag(email, flag, wanted),
  )

  // A clicked notification: one message opens in the reader (wherever the list
  // is), several show that account's inbox. Reads the latest tree through a ref
  // so the subscription does not restart on every tree change.
  const latestTree = useRef({ accounts, folders })
  useEffect(() => { latestTree.current = { accounts, folders } }, [accounts, folders])
  const [removeError, setRemoveError] = useState<string | null>(null)
  // A message that left the view lets go of the reader. When a keyboard shortcut
  // archived or deleted the opened message, its neighbour in the list opens instead.
  const advanceAfter = useRef<{ id: string; next: EmailSummary | null } | null>(null)
  const openedRef = useRef(openedState)
  useEffect(() => { openedRef.current = openedState }, [openedState])
  const openRef = useRef<(email: EmailSummary) => void>(() => undefined)
  const letGo = useCallback((email: { id: string }) => {
    const follow = advanceAfter.current?.id === email.id ? advanceAfter.current : null
    if (follow) advanceAfter.current = null
    if (follow?.next && openedRef.current?.id === email.id) openRef.current(follow.next)
    else setOpened((current) => current?.id === email.id ? null : current)
  }, [])
  const removeLocalCopy = useCallback((email: { id: string }) => {
    setRemoveError(null)
    client.removeLocalCopy(email.id).then(() => {
      letGo(email)
      void queryClient.invalidateQueries({ queryKey: mailKeys.emailLists() })
    }).catch((error: unknown) => {
      const message = errorText(error)
      const readerCanShowError = openedState?.id === email.id && !readerCollapsed
      if (readerCanShowError) setRemoveError(message)
      else {
        toast.error(t('mail:removeLocalCopyFailed'), { description: message })
      }
    })
  }, [client, letGo, openedState, queryClient, readerCollapsed])
  const [removeLocalChoice, setRemoveLocalChoice] = useState<EmailSummary | null>(null)
  const requestRemoveLocal = useCallback((email: EmailSummary) => setRemoveLocalChoice(email), [setRemoveLocalChoice])
  const confirmRemoveLocal = useCallback(() => {
    const email = removeLocalChoice
    setRemoveLocalChoice(null)
    if (email) removeLocalCopy(email)
  }, [removeLocalChoice, removeLocalCopy, setRemoveLocalChoice])
  const deleteEmail = useDeleteEmail(client, folders, letGo)
  const archiveEmail = useArchiveEmail(client, letGo)
  const hideOpened = useCallback((email: { id: string }) => {
    setOpened((current) => current?.id === email.id ? null : current)
  }, [])
  const archiveEmails = useArchiveEmails(client, hideOpened)
  const selectionActions = useSelectionActions({
    client, selection, listQuery: selectQuery, folders, archivedView: archivedView && !contactView,
    deleteMany: deleteEmail.requestBatch,
    archiveMany: archiveEmails,
    flagMany: setFlags,
  })
  useEffect(() => client.onNotificationOpen(({ accountId, emailId, outboxId }) => {
    if (outboxId) {
      setOutboxChoice(outboxId)
      showMailbox({ kind: 'outbox' })
      return
    }
    const { accounts, folders } = latestTree.current
    const account = accounts.find((row) => row.id === accountId)
    if (!account) return
    if (emailId) {
      setOpened({ id: emailId, accountId, source: account.mailboxSourceId })
      setAnchorId(emailId)
      setReaderChoice(false)
      exitSelection()
      return
    }
    const inbox = folders.find((folder) => folder.accountId === accountId && folder.role === 'inbox' && folder.selectable)
    if (inbox) {
      setSearch('')
      setFilter('all')
      showMailbox({ kind: 'folder', folder: inbox })
    }
  }), [client, exitSelection, showMailbox])

  // Cmd/Ctrl+N writes a new mail from anywhere in the main window.
  const composeNew = compose.compose
  useSettingsRequests(setSettings)
  useDesktopActions((action) => {
    if (action === 'settings') setSettings({ section: 'general' })
    else if (action === 'compose') void composeNew()
  })
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

  const contactRules = useContactAggregation(client)
  const baseTitle = mailboxTitle(activeMailbox)
  // Into the contact view: remember where the list was, then slide to the sender's mail.
  const viewContact = useCallback((email: EmailSummary) => {
    if (!contact) savedScroll.current = liveScroll.current
    setContact({
      address: email.from.address.trim(),
      name: email.from.name?.trim() || null,
      accountId: selectedAccountId,
    })
    setSlide('forward')
  }, [contact, selectedAccountId])
  const leaveContact = useCallback(() => {
    setRestoreScroll(savedScroll.current)
    setContact(null)
    setSlide('back')
  }, [])
  // Esc and the mouse's back button return from the contact view. Esc yields to selection
  // mode, to an open message (the reader closes first) and to anything else that uses it.
  const inContact = contactView !== null
  const escapeGoesBack = inContact && !selecting && !(opened && !readerCollapsed)
  useEffect(() => {
    if (!escapeGoesBack) return
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || escapeBelongsToSomethingElse(event)) return
      event.preventDefault()
      leaveContact()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [escapeGoesBack, leaveContact])
  useEffect(() => {
    if (!inContact) return
    const onMouseUp = (event: MouseEvent) => {
      if (event.button !== 3) return
      event.preventDefault()
      leaveContact()
    }
    window.addEventListener('mouseup', onMouseUp)
    return () => window.removeEventListener('mouseup', onMouseUp)
  }, [inContact, leaveContact])

  const loadMore = () => {
    if (list.hasNextPage && !list.isFetching) void list.fetchNextPage()
  }

  // A listed message opened in the Unread view stays in it once read; see `keptIds`.
  const keepListed = (id: string) => {
    if (filter !== 'unread' || !emails.some((email) => email.id === id)) return
    setKept((current) => {
      const ids = current.view === viewKey ? current.ids : NO_IDS
      return ids.includes(id) ? current : { view: viewKey, ids: [...ids, id].slice(-MAX_KEPT) }
    })
  }
  // Every way of opening a message (list click, search result, notification) shows the reader.
  const openEmail = (email: { id: string; accountId: string }) => {
    keepListed(email.id)
    setOpened({ id: email.id, accountId: email.accountId, source: source(email.accountId) })
    setAnchorId(email.id)
    setReaderCollapsed(false)
    // Opening a message (from search or the palette) ends picking, so the reader can show it.
    exitSelection()
  }
  useEffect(() => { openRef.current = openEmail })

  // Single-key shortcuts (see `shortcuts.ts`). Browsing moves the opened message
  // like a click on the next or previous row; group rows are not messages and are
  // skipped. Acting needs the opened message on screen, with the reader toolbar's rules.
  const singleKeys = settingsQuery.data?.settings.general.singleKeyShortcuts ?? true
  const [helpOpen, setHelpOpen] = useState(false)
  const [reveal, setReveal] = useState<RevealRequest | null>(null)
  const runShortcut = (id: SingleKeyShortcutId): boolean => {
    if (id === 'help') {
      setHelpOpen(true)
      return true
    }
    if (id === 'search') {
      setPaletteOpen(true)
      return true
    }
    if (!mailView) return false
    if (selecting) {
      // Picking: the flags, archive and delete go to the picked messages, as in the header.
      const state = selectionAvailability(selection, selectionActions)
      if (state.none) return false
      if (id === 'archive' && !state.archiveBlocked) selectionActions.onArchive()
      else if (id === 'delete' && state.deletable > 0) selectionActions.onDelete()
      else if (id === 'star' && state.changeable > 0) selectionActions.onFlag('starred', state.anyUnstarred)
      else if (id === 'read' && state.changeable > 0) selectionActions.onFlag('read', state.anyUnread)
      else return false
      return true
    }
    const rows = emails.filter((email) => !email.senderGroup)
    const own = opened ? rows.findIndex((email) => email.id === opened.id) : -1
    // A message expanded inside a thread may not be listed; its thread's row is.
    const at = own >= 0 ? own : opened && anchorId ? rows.findIndex((email) => email.id === anchorId) : -1
    const go = (email: EmailSummary | undefined) => {
      if (!email) return false
      openEmail(email)
      setReveal((previous) => ({ id: email.id, seq: (previous?.seq ?? 0) + 1 }))
      return true
    }
    if (id === 'next') {
      if (at >= 0 && at === rows.length - 1) loadMore()
      return go(at < 0 ? rows[0] : rows[at + 1])
    }
    if (id === 'previous') return go(at < 0 ? rows[0] : rows[at - 1])
    if (id === 'open') {
      if (!opened) return go(rows[0])
      if (!readerCollapsed) return false
      setReaderCollapsed(false)
      return true
    }
    const summary = opened && !readerCollapsed && body.email?.summary.id === opened.id ? body.email.summary : null
    if (!summary) return false
    // After archive or delete, the next row opens (the previous one at the end of the list).
    const advance = () => {
      advanceAfter.current = { id: summary.id, next: own >= 0 ? rows[own + 1] ?? rows[own - 1] ?? null : null }
    }
    switch (id) {
      case 'archive':
        if (!canArchive(summary, folders)) return false
        advance()
        void archiveEmail(summary, !summary.archived)
        return true
      case 'delete':
        advance()
        if (summary.localOnly) requestRemoveLocal(summary)
        else deleteEmail.request(summary)
        return true
      case 'star':
      case 'read': {
        if (summary.localOnly) return false
        const flag = id === 'star' ? 'starred' : 'read'
        void setFlag(summary, flag, !(flag === 'starred' ? summary.isStarred : summary.isRead))
        return true
      }
      case 'reply':
      case 'replyAll':
      case 'forward':
        void compose.respond(summary.id, id)
        return true
    }
  }
  useMailShortcuts(singleKeys, runShortcut)

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
        <NoAccounts onAddAccount={() => setAddingAccount(true)} onSettings={() => setSettings({ section: 'general' })} />
        {dialog}
      </>
    )
  }
  const showAccountBadges = (activeMailbox.kind === 'unified' || activeMailbox.kind === 'starred' || (activeMailbox.kind === 'archived' && !activeMailbox.accountId)) && accounts.length > 1
  // Drafts and outbox are not searched; the title-bar search then speaks for the inbox.
  const searchScope = !mailView ? mailboxTitle({ kind: 'unified', role: 'inbox' }) : contactView
    ? contactView.name ?? contactView.address
    : activeMailbox.kind === 'folder'
    ? `${accounts.find((account) => account.id === activeMailbox.folder.accountId)?.displayName ?? ''} · ${folderLabel(activeMailbox.folder)}`
    : activeMailbox.kind === 'archived' && activeMailbox.accountId
      ? `${accounts.find((account) => account.id === activeMailbox.accountId)?.displayName ?? ''} · ${mailboxTitle(activeMailbox)}`
      : mailboxTitle(activeMailbox)
  const firstPageError = list.isError && !list.isFetchNextPageError ? errorText(list.error) : null

  const selectMailbox = showMailbox
  // A search or a found message belongs to a mail list: leave the local views for it.
  const leaveLocalView = () => { if (!mailView) setMailbox({ kind: 'unified', role: 'inbox' }) }
  const selectedDraft = drafts.data?.some((draft) => draft.id === draftChoice) ? draftChoice : null
  const selectedOutbox = outbox.data?.find((item) => item.id === outboxChoice) ?? null

  // Expanding a message inside the thread opens it like any other (read mark, toolbar,
  // body) but keeps the anchor, so the same conversation stays on screen.
  const expandInThread = (email: EmailSummary) => {
    keepListed(email.id)
    setOpened({ id: email.id, accountId: email.accountId, source: source(email.accountId) })
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

  // What the full sidebar and its icon rail both show.
  const sidebarProps = {
    accounts: displayAccounts,
    folders,
    syncStates: currentSyncStates,
    mailbox: activeMailbox,
    onSelect: selectMailbox,
    onOpenAccounts: () => setAccountsTarget({}),
    onOpenSettings: () => setSettings({ section: 'general' }),
    onCompose: () => void compose.compose(),
    draftCount,
    outboxCount,
  }

  return (
    <TooltipProvider>
      <AvatarSourceProvider client={client} enabled={senderAvatars}>
      <ShellLayout
        navLabel={t('mail:mailboxNavigation')}
        navWidth={navWidth}
        navCollapsed={navCollapsed}
        navActions={<RefreshAllButton accounts={accounts} syncStates={currentSyncStates} onRefresh={(accountId) => client.refreshSync(accountId)} />}
        onNavChange={saveLayout}
        nav={<MailSidebar {...sidebarProps} loading={treeLoading} onRefresh={(accountId) => client.refreshSync(accountId)} />}
        rail={<MailRail {...sidebarProps} />}
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
          scopeQuery={mailView ? {
            accountIds: listQuery.accountIds,
            folderId: listQuery.folderId,
            folderRole: listQuery.folderRole,
            archive: listQuery.archive,
            starred: listQuery.starred,
            excludeFolderRoles: listQuery.excludeFolderRoles,
            fromAddress: listQuery.fromAddress,
          } : { folderRole: 'inbox' }}
          sourceIdentity={sourceIdentity}
          accounts={displayAccounts}
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
          translation={mailView && !selecting && !readerCollapsed && translation.available ? translation : null}
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
            // The whole pane content slides as one piece on a base <-> contact switch; the
            // key replays it per view. Never the virtual rows themselves.
            <div
              key={contactView ? `contact:${contactView.address.toLowerCase()}` : 'base'}
              data-slot="list-pane"
              className={cn('h-full min-h-0', slide && !reducedMotion && listSlideClass(slide))}
            >
            <EmailList
              onRequestPreviews={requestPreviews}
              title={contactView ? contactView.name ?? contactView.address : baseTitle}
              accountName={accounts.find((account) => account.id === selectedAccountId)?.displayName}
              contact={contactView ? { name: contactView.name, address: contactView.address, backTitle: baseTitle, onBack: leaveContact } : undefined}
              initialScrollTop={contactView ? undefined : restoreScroll}
              onScrollOffset={contactView ? undefined : trackScroll}
              onViewContact={viewContact}
              onAggregate={contactRules.setAggregated}
              isAggregated={contactRules.isAggregated}
              sentView={sentView}
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
              indexedBodies={pages?.at(-1)?.coverage.indexedBodies ?? 0}
              selectedId={highlighted}
              previewLines={previewLines}
              mailboxLabel={mailboxLabel}
              onFilterChange={setFilter}
              onSelect={openEmail}
              onFlag={(email, flag, wanted) => void setFlag(email, flag, wanted)}
              onArchive={(email, archive) => void archiveEmail(email, archive)}
              onDelete={deleteEmail.request}
              onRemoveLocal={requestRemoveLocal}
              canArchive={(email) => canArchive(email, folders)}
              onRespond={(email, kind) => void compose.respond(email.id, kind)}
              onLoadMore={loadMore}
              selection={selection}
              selectionActions={selectionActions}
              reveal={reveal}
            />
            </div>
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
            ) : selecting ? (
              <SelectionSummary items={selection.items} accounts={displayAccounts} />
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
              error={opened && body.detail.isError ? errorText(body.detail.error) : removeError}
              downloading={body.pending}
              onDownload={body.download}
              onRespond={(kind) => opened && void compose.respond(opened.id, kind)}
              onRemoveLocal={removeLocalCopy}
              translation={translation}
              onDelete={deleteEmail.request}
              frame={threaded && threadPage && opened ? (content) => (
                <ConversationFrame
                  // A new thread starts with its earlier / newer rows folded again.
                  key={anchorId}
                  page={threadPage}
                  expandedId={opened.id}
                  selfAddress={accounts.find((account) => account.id === opened.accountId)?.emailAddress ?? null}
                  mode={threadMode}
                  onModeChange={setThreadMode}
                  onExpand={expandInThread}
                >{content}</ConversationFrame>
              ) : undefined}
              onArchive={body.email && canArchive(body.email.summary, folders) ? (email, archive) => void archiveEmail(email, archive) : undefined}
            />
            )}
          </ResizablePanel>
        </ResizablePanelGroup>
        )}
        {accounts.length > 0 && <ReaderToggle collapsed={readerCollapsed} onToggle={toggleReader} />}
      </ShellLayout>
      </AvatarSourceProvider>
      {dialog}
      <TranslationConsentDialog translation={translation} />
      <ShortcutHelpDialog
        open={helpOpen}
        onOpenChange={setHelpOpen}
        onOpenSettings={() => {
          setHelpOpen(false)
          setSettings({ section: 'shortcuts' })
        }}
      />
      <DeleteForGoodDialog {...deleteEmail.confirmation} />
      <RemoveLocalCopyDialog
        open={removeLocalChoice !== null}
        onOpenChange={(open) => { if (!open) setRemoveLocalChoice(null) }}
        onConfirm={confirmRemoveLocal}
      />
    </TooltipProvider>
  )
}

/** Entering a contact slides the list in from the right, coming back from the left. */
function listSlideClass(slide: 'forward' | 'back') {
  return cn(
    'animate-in fade-in-0 duration-200 ease-out',
    slide === 'forward' ? 'slide-in-from-right-6' : 'slide-in-from-left-6',
  )
}

function NoAccounts({ onAddAccount, onSettings }: { onAddAccount: () => void; onSettings: () => void }) {
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
        <Button variant="ghost" onClick={onSettings}>{t('settings:title')}</Button>
      </EmptyContent>
    </Empty>
  )
}
