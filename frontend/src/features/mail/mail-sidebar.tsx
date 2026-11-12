import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { errorText } from '@/i18n/errors'
import { Fragment, useEffect, useRef, useState } from 'react'
import { HugeiconsIcon, type IconSvgElement } from '@hugeicons/react'
import {
  Alert02Icon,
  Archive03Icon,
  ArrowRight01Icon,
  AtSignIcon,
  Delete02Icon,
  File01Icon,
  InboxIcon,
  MailUpload01Icon,
  MoreHorizontalIcon,
  PencilEdit02Icon,
  RefreshIcon,
  SentIcon,
  Settings01Icon,
  SpamIcon,
  StarIcon,
} from '@hugeicons/core-free-icons'
import type { Account, AccountSyncState, Folder, FolderRole, SyncFailure } from '@/generated/accounts'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible'
import {
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuAction,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSkeleton,
  SidebarMenuSub,
} from '@/components/ui/sidebar'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { useDelayedFlag } from '@/hooks/use-delayed-flag'
import { cn } from '@/lib/utils'
import { buildFolderTree, countedUnread, loadCollapsed, loadExpanded, saveCollapsed, saveExpanded, type FolderNode } from './folder-tree'
import { folderLabel, type Mailbox } from './mailbox'
import { AccountAvatar } from '@/features/accounts/account-avatar'
import { formatFullTime } from './format'
import { useManualSync } from './use-manual-sync'
import { COMPOSE_SHORTCUT, ROLE_ICONS, type MailSidebarProps } from './sidebar-shared'

export function MailSidebar({ accounts, folders, loading, syncStates, mailbox, onSelect, onOpenAccounts, onOpenSettings, onCompose, draftCount, outboxCount, onRefresh }: MailSidebarProps) {
  useTranslation()
  const [collapsed, setCollapsed] = useState(loadCollapsed)
  const toggle = (id: string, open: boolean) => {
    const next = new Set(collapsed)
    if (open) next.delete(id)
    else next.add(id)
    setCollapsed(next)
    saveCollapsed(next)
  }

  const scroller = useRef<HTMLDivElement>(null)
  const [fade, setFade] = useState({ top: false, bottom: false })
  const updateFade = () => {
    const el = scroller.current
    if (!el) return
    const top = el.scrollTop > 1
    const bottom = el.scrollTop + el.clientHeight < el.scrollHeight - 1
    setFade((prev) => (prev.top === top && prev.bottom === bottom ? prev : { top, bottom }))
  }
  // The list hides its scrollbar, so a fade at each clipped edge is the only hint that more lies beyond.
  useEffect(() => {
    const el = scroller.current
    if (!el) return
    updateFade()
    const observer = new ResizeObserver(updateFade)
    observer.observe(el)
    for (const child of el.children) observer.observe(child)
    return () => observer.disconnect()
  }, [])

  // Accounts and top rows start closed; `expanded` is what the user opened and is
  // remembered. `revealed` is opened for this session only, to show a selection
  // made elsewhere (command palette, notification); closing a row clears both.
  const [expanded, setExpanded] = useState(loadExpanded)
  const [revealed, setRevealed] = useState<Set<string>>(() => new Set())
  const isOpen = (id: string) => expanded.has(id) || revealed.has(id)
  const setOpen = (id: string, open: boolean) => {
    const next = new Set(expanded)
    if (open) next.add(id)
    else next.delete(id)
    setExpanded(next)
    saveExpanded(next)
    if (!open && revealed.has(id)) setRevealed((current) => new Set([...current].filter((item) => item !== id)))
  }

  const inboxes = folders.filter((folder) => folder.selectable && folder.role === 'inbox')
  const unifiedUnread = inboxes.reduce((total, folder) => total + folder.unreadCount, 0)
  const unifiedCountsKnown = inboxes.every((folder) => folder.countsKnown)
  // Locally archived inbox mail (not the server's archive folder), counted apart from the inbox number.
  const archivedUnread = inboxes.reduce((total, folder) => total + folder.archivedUnreadCount, 0)
  const junkUnread = folders.some((folder) => folder.selectable && folder.role === 'junk' && folder.countsKnown && folder.unreadCount > 0)
  const selectedId = mailbox.kind === 'folder' ? mailbox.folder.id : null
  const hasSyncFailure = accounts.some((account) => syncStates[account.id]?.failure)

  /** Each account's folder of this role, in the user's account order. */
  const byAccount = (role: FolderRole): AccountChild[] => accounts.flatMap((account) => {
    const folder = folders.find((item) => item.accountId === account.id && item.selectable && item.role === role)
    if (!folder) return []
    const badge = role === 'inbox' ? countBadge(folder.unreadCount, folder.countsKnown)
      : role === 'junk' && folder.countsKnown && folder.unreadCount > 0 ? <UnreadDot /> : null
    return [{ account, badge, active: folder.id === selectedId, mailbox: { kind: 'folder', folder } }]
  })
  const archivedByAccount: AccountChild[] = accounts.flatMap((account) => {
    const inbox = inboxes.find((folder) => folder.accountId === account.id)
    if (!inbox) return []
    return [{
      account,
      badge: countBadge(inbox.archivedUnreadCount, true),
      active: mailbox.kind === 'archived' && mailbox.accountId === account.id,
      mailbox: { kind: 'archived', accountId: account.id },
    }]
  })
  const isUnified = (role: FolderRole) => mailbox.kind === 'unified' && mailbox.role === role
  const roleRow = (role: FolderRole, icon: IconSvgElement, label: string, badge: React.ReactNode = null) => (
    <TopRow
      icon={icon} label={label} badge={badge}
      active={isUnified(role)} onSelect={() => onSelect({ kind: 'unified', role })}
      accounts={byAccount(role)} open={isOpen(`role:${role}`)} onOpenChange={(next) => setOpen(`role:${role}`, next)} onSelectChild={onSelect}
    />
  )
  const moreRows: [string, () => React.ReactNode][] = [
    ['archived', () => (
      <TopRow
        icon={Archive03Icon} label={t('mail:archive')} badge={countBadge(archivedUnread, true)}
        active={mailbox.kind === 'archived' && !mailbox.accountId} onSelect={() => onSelect({ kind: 'archived', accountId: null })}
        accounts={archivedByAccount} open={isOpen('role:archived')} onOpenChange={(next) => setOpen('role:archived', next)} onSelectChild={onSelect}
      />
    )],
    ['junk', () => roleRow('junk', SpamIcon, t('mail:junk'), junkUnread ? <UnreadDot /> : null)],
    ['trash', () => roleRow('trash', Delete02Icon, t('mail:trash'))],
  ]
  const moreOpen = isOpen('more')

  // When the selection changes, open what it takes to show it, unless it is in sight
  // already: a view under 更多 opens 更多; a folder opens its account, unless the
  // top row it belongs to lists it.
  const shownUnderTop = (role: FolderRole, children: AccountChild[]) => {
    const rowId = role === 'inbox' || role === 'sent' || role === 'junk' || role === 'trash' ? `role:${role}` : null
    return rowId !== null && children.length > 1 && isOpen(rowId) && (role === 'inbox' || role === 'sent' || moreOpen)
  }
  const reveal: string[] = mailbox.kind === 'unified' && (mailbox.role === 'junk' || mailbox.role === 'trash') ? ['more']
    : mailbox.kind === 'archived' && !mailbox.accountId ? ['more']
      : mailbox.kind === 'folder' && !shownUnderTop(mailbox.folder.role, byAccount(mailbox.folder.role)) ? [`account:${mailbox.folder.accountId}`]
        : mailbox.kind === 'archived' && mailbox.accountId && !(archivedByAccount.length > 1 && isOpen('role:archived') && moreOpen) ? [`account:${mailbox.accountId}`]
          : []
  const selectionKey = JSON.stringify([mailbox.kind, mailbox.kind === 'folder' ? mailbox.folder.id : mailbox.kind === 'archived' ? mailbox.accountId : mailbox.kind === 'unified' ? mailbox.role : null])
  // `null` first, so a selection the sidebar starts with is revealed as well.
  const [revealedFor, setRevealedFor] = useState<string | null>(null)
  if (revealedFor !== selectionKey) {
    // Adjusting state during render: React re-renders before painting, so nothing flickers.
    setRevealedFor(selectionKey)
    const missing = reveal.filter((id) => !isOpen(id))
    if (missing.length > 0) setRevealed((current) => new Set([...current, ...missing]))
  }

  return (
    <div data-slot="mail-sidebar" className="flex h-full min-h-0 flex-col">
      <SidebarHeader>
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton onClick={onCompose} className="font-medium" title={t('mail:composeShortcut', { shortcut: COMPOSE_SHORTCUT })}>
              <HugeiconsIcon icon={PencilEdit02Icon} />
              <span>{t('mail:compose')}</span>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarHeader>
      <SidebarContent ref={scroller} onScroll={updateFade} style={fadeMask(fade)}>
        <SidebarGroup>
          <SidebarGroupContent>
            <SidebarMenu>
              {roleRow('inbox', InboxIcon, t('mail:inbox'), unifiedCountsKnown ? countBadge(unifiedUnread, true) : countBadge(0, false))}
              <SidebarMenuItem>
                <SidebarMenuButton isActive={mailbox.kind === 'starred'} onClick={() => onSelect({ kind: 'starred' })}>
                  <HugeiconsIcon icon={StarIcon} />
                  <span>{t('mail:starredView')}</span>
                </SidebarMenuButton>
              </SidebarMenuItem>
              <SidebarMenuItem>
                {/* Drafts never clear themselves, so a standing number would turn into
                    noise. The count stays in the tooltip and the drafts view's own header. */}
                <SidebarMenuButton
                  isActive={mailbox.kind === 'drafts'} onClick={() => onSelect({ kind: 'drafts' })}
                  title={draftCount > 0 ? `${t('mail:drafts')} · ${t('mail:draftCount', { count: draftCount })}` : undefined}
                >
                  <HugeiconsIcon icon={File01Icon} />
                  <span>{t('mail:drafts')}</span>
                </SidebarMenuButton>
              </SidebarMenuItem>
              {roleRow('sent', SentIcon, t('mail:sent'))}
              {outboxCount > 0 && (
                <SidebarMenuItem>
                  <SidebarMenuButton isActive={mailbox.kind === 'outbox'} onClick={() => onSelect({ kind: 'outbox' })}>
                    <HugeiconsIcon icon={MailUpload01Icon} />
                    <span>{t('mail:outbox')}</span>
                  </SidebarMenuButton>
                  <SidebarMenuBadge aria-label={t('mail:outboxCount', { count: outboxCount })}>{outboxCount}</SidebarMenuBadge>
                </SidebarMenuItem>
              )}
              <Collapsible open={moreOpen} onOpenChange={(next) => setOpen('more', next)} render={<SidebarMenuItem />}>
                <CollapsibleTrigger render={<SidebarMenuButton />}>
                  <HugeiconsIcon icon={MoreHorizontalIcon} />
                  <span className="flex-1">{t('mail:more')}</span>
                  <HugeiconsIcon icon={ArrowRight01Icon} className={cn('transition-transform motion-reduce:transition-none', moreOpen && 'rotate-90')} />
                </CollapsibleTrigger>
                <CollapsibleContent>
                  <SidebarMenuSub>
                    {moreRows.map(([key, row]) => <Fragment key={key}>{row()}</Fragment>)}
                  </SidebarMenuSub>
                </CollapsibleContent>
              </Collapsible>
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
        <SidebarGroup>
          <SidebarGroupLabel>{t('mail:mailboxes')}</SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              {loading &&
                Array.from({ length: 4 }, (_, index) => (
                  <SidebarMenuItem key={index}>
                    <SidebarMenuSkeleton showIcon />
                  </SidebarMenuItem>
                ))}
              {accounts.map((account) => {
                const own = folders.filter((folder) => folder.accountId === account.id)
                const tree = buildFolderTree(own)
                const unread = tree.reduce((total, node) => total + node.unreadWithChildren, 0)
                const countsKnown = tree.every((node) => node.countsKnown)
                const sync = syncStates[account.id]
                // Room for the one shared slot (and the warning beside it), never more.
                const rowStyle = { paddingRight: `calc(${Math.max(2, String(unread).length)}ch + ${sync?.failure ? 2.5 : 1}rem)` }
                const status = (
                  <>
                    <AccountSyncSlot
                      key={account.mailboxSourceId}
                      unread={unread}
                      countsKnown={countsKnown}
                      sync={sync}
                      label={account.displayName}
                      onRefresh={() => onRefresh(account.id)}
                      className={cn(sync?.failure && 'right-7')}
                    />
                    {sync?.failure && <SyncFailureAction failure={sync.failure} />}
                  </>
                )
                const avatar = <AccountAvatar displayName={account.displayName} color={account.color} providerId={account.providerId} useProviderIcon={account.useProviderIcon} className="size-[18px] text-[10px]" />
                // Before its first folder listing an account has nothing to fold: say why it is empty.
                if (own.length === 0) {
                  return (
                    <SidebarMenuItem key={account.id}>
                      <SidebarMenuButton render={<div />} title={account.emailAddress} className="font-medium hover:bg-transparent active:bg-transparent" style={rowStyle}>
                        <span aria-hidden className="size-4 shrink-0" />
                        {avatar}
                        <span>{account.displayName}</span>
                      </SidebarMenuButton>
                      {status}
                      <SidebarMenuSub>
                        <SidebarMenuItem>
                          <span className="flex h-8 items-center gap-2 px-2 text-xs text-sidebar-foreground/70">
                            {sync?.phase === 'failed' || sync?.phase === 'idle' ? t('mail:noFolders') : (
                              <span role="status">{t('mail:syncingFolders')}</span>
                            )}
                          </span>
                        </SidebarMenuItem>
                      </SidebarMenuSub>
                    </SidebarMenuItem>
                  )
                }
                const open = isOpen(`account:${account.id}`)
                return (
                  <Collapsible
                    key={account.id}
                    open={open}
                    onOpenChange={(next) => setOpen(`account:${account.id}`, next)}
                    render={<SidebarMenuItem />}
                  >
                    <CollapsibleTrigger render={<SidebarMenuButton title={account.emailAddress} className="font-medium" style={rowStyle} />}>
                      <HugeiconsIcon icon={ArrowRight01Icon} className={cn('transition-transform motion-reduce:transition-none', open && 'rotate-90')} />
                      {avatar}
                      <span>{account.displayName}</span>
                    </CollapsibleTrigger>
                    {status}
                    <CollapsibleContent>
                      <SidebarMenuSub>
                        {tree.map((node) => (
                          <Fragment key={node.folder.id}>
                            <FolderItem
                              node={node}
                              selectedId={selectedId}
                              collapsed={collapsed}
                              onToggle={toggle}
                              onSelect={(folder) => onSelect({ kind: 'folder', folder })}
                            />
                            {node.folder.role === 'inbox' && node.folder.selectable && (
                              <SidebarMenuItem>
                                <SidebarMenuButton
                                  isActive={mailbox.kind === 'archived' && mailbox.accountId === account.id}
                                  onClick={() => onSelect({ kind: 'archived', accountId: account.id })}
                                >
                                  <HugeiconsIcon icon={Archive03Icon} />
                                  <span>{t('mail:archivedFolder')}</span>
                                </SidebarMenuButton>
                                {node.folder.archivedUnreadCount > 0 && <SidebarMenuBadge>{node.folder.archivedUnreadCount}</SidebarMenuBadge>}
                              </SidebarMenuItem>
                            )}
                          </Fragment>
                        ))}
                      </SidebarMenuSub>
                    </CollapsibleContent>
                  </Collapsible>
                )
              })}
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
      </SidebarContent>
      <SidebarFooter>
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton onClick={onOpenAccounts}>
              <HugeiconsIcon icon={AtSignIcon} />
              <span>{t('mail:accounts')}</span>
            </SidebarMenuButton>
            {hasSyncFailure && <SidebarMenuBadge aria-label={t('mail:someAccountsSyncFailed')} className="text-destructive">!</SidebarMenuBadge>}
          </SidebarMenuItem>
          <SidebarMenuItem>
            <SidebarMenuButton onClick={onOpenSettings}>
              <HugeiconsIcon icon={Settings01Icon} />
              <span>{t('mail:settings')}</span>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarFooter>
    </div>
  )
}

/** One account's entry under a top row such as 收件箱. */
type AccountChild = { account: Account; badge: React.ReactNode; active: boolean; mailbox: Mailbox }

/** An unread number, "—" while unknown, or nothing for none. */
function countBadge(count: number, known: boolean): React.ReactNode {
  if (!known) return <span aria-label={t('mail:unreadCountUnknown')}>—</span>
  return count > 0 ? count : null
}

/** Junk signals unread mail with a dot, never a number. */
function UnreadDot() {
  return (
    <span aria-label={t('mail:hasUnread')} className="flex items-center justify-center">
      <span className="size-1.5 rounded-full bg-current" />
    </span>
  )
}

/**
 * A view across all accounts. With two or more accounts a chevron, shown on
 * hover or focus and while open, lists each account's own folder below it;
 * the badge slides left to make room rather than being covered. `--show` is set
 * by this row's own button or slot only, since the item holds its children too.
 */
function TopRow({ icon, label, badge, active, onSelect, accounts, open: wanted, onOpenChange, onSelectChild }: {
  icon: IconSvgElement
  label: string
  badge: React.ReactNode
  active: boolean
  onSelect: () => void
  accounts: AccountChild[]
  open: boolean
  onOpenChange: (open: boolean) => void
  onSelectChild: (mailbox: Mailbox) => void
}) {
  useTranslation()
  const expandable = accounts.length > 1
  const open = expandable && wanted
  return (
    <Collapsible open={open} onOpenChange={onOpenChange} render={<SidebarMenuItem />}>
      <SidebarMenuButton isActive={active} onClick={onSelect} className={cn(expandable ? (badge != null ? 'pr-14' : 'pr-8') : badge != null && 'pr-8')}>
        <HugeiconsIcon icon={icon} />
        <span>{label}</span>
      </SidebarMenuButton>
      {(badge != null || expandable) && (
        <div
          className={cn(
            'pointer-events-none absolute top-1.5 right-1 flex h-5 items-center text-xs font-medium tabular-nums text-sidebar-foreground select-none group-data-[collapsible=icon]:hidden',
            // A selected row keeps light text when hovered too, like `SidebarMenuBadge`.
            'peer-hover/menu-button:text-sidebar-accent-foreground peer-data-active/menu-button:text-primary-foreground peer-data-active/menu-button:peer-hover/menu-button:text-primary-foreground',
            open ? '[--show:1]' : '[--show:0] peer-hover/menu-button:[--show:1] hover:[--show:1] focus-within:[--show:1]',
          )}
        >
          {badge != null && <span className="flex min-w-5 items-center justify-center px-1">{badge}</span>}
          {expandable && (
            <CollapsibleTrigger
              render={(
                <button
                  type="button"
                  aria-label={t('mail:toggleAccounts', { action: open ? t('mail:collapse') : t('mail:expand'), folder: label })}
                  className={cn(
                    'pointer-events-auto flex h-5 w-[calc(1.25rem*var(--show))] shrink-0 items-center justify-center overflow-hidden rounded-md opacity-(--show) outline-hidden transition-[width,opacity] motion-reduce:transition-none focus-visible:ring-2 focus-visible:ring-sidebar-ring',
                    // On a selected row, a light tint like `SidebarMenuAction`, not the light hover fill.
                    active ? 'hover:bg-primary-foreground/15 hover:text-primary-foreground' : 'hover:bg-sidebar-accent hover:text-sidebar-accent-foreground',
                  )}
                />
              )}
            >
              <HugeiconsIcon icon={ArrowRight01Icon} className={cn('size-4 shrink-0 transition-transform motion-reduce:transition-none', open && 'rotate-90')} />
            </CollapsibleTrigger>
          )}
        </div>
      )}
      {expandable && (
        <CollapsibleContent>
          <SidebarMenuSub>
            {accounts.map((child) => (
              <SidebarMenuItem key={child.account.id}>
                <SidebarMenuButton isActive={child.active} title={child.account.emailAddress} onClick={() => onSelectChild(child.mailbox)} className={cn(child.badge != null && 'pr-8')}>
                  <AccountAvatar displayName={child.account.displayName} color={child.account.color} providerId={child.account.providerId} useProviderIcon={child.account.useProviderIcon} className="size-4 text-[9px]" />
                  <span>{child.account.displayName}</span>
                </SidebarMenuButton>
                {child.badge != null && <SidebarMenuBadge>{child.badge}</SidebarMenuBadge>}
              </SidebarMenuItem>
            ))}
          </SidebarMenuSub>
        </CollapsibleContent>
      )}
    </Collapsible>
  )
}

const FADE_SIZE = '1.75rem'

/** Fades the list out toward each edge that has more content behind it. */
function fadeMask({ top, bottom }: { top: boolean; bottom: boolean }): React.CSSProperties | undefined {
  if (!top && !bottom) return undefined
  const mask = `linear-gradient(to bottom, ${top ? `transparent, black ${FADE_SIZE}` : 'black'}, ${bottom ? `black calc(100% - ${FADE_SIZE}), transparent` : 'black'})`
  return { maskImage: mask, WebkitMaskImage: mask }
}

/** Only a user-requested refresh animates this glyph. */
function SyncIcon({ spinning, className }: { spinning: boolean; className?: string }) {
  return (
    <HugeiconsIcon
      icon={RefreshIcon}
      strokeWidth={2}
      className={cn(className, spinning && 'animate-spin [animation-duration:1.4s] motion-reduce:animate-none')}
    />
  )
}

/**
 * One slot at the end of the account row, shared by the unread count, the
 * refresh button and the spinner of a refresh the user started. The count
 * shows at rest; hovering or focusing this row puts the refresh button in its
 * place, and a manual refresh keeps the turning button there until it ends.
 * Automatic rounds never animate or announce a status, so the count stays.
 * `--show` is local to this row because the account's `li` also contains its
 * whole folder subtree.
 */
function AccountSyncSlot({ unread, countsKnown, sync, label, onRefresh, className }: {
  unread: number
  countsKnown: boolean
  sync: AccountSyncState | undefined
  label: string
  onRefresh: () => Promise<void>
  className?: string
}) {
  useTranslation()
  const { refreshing, refresh } = useManualSync(sync, onRefresh)
  const turning = useDelayedFlag(refreshing, 400, 700)
  const busy = sync?.phase === 'syncing' || refreshing || turning
  const showCount = !countsKnown || unread > 0
  const actionLabel = busy ? t('mail:syncingAccount', { label }) : t('mail:syncAccount', { label })
  return (
    <div
      data-slot="account-sync"
      className={cn(
        'absolute top-1.5 right-1 flex h-5 min-w-5 items-center justify-center text-xs font-medium tabular-nums text-sidebar-foreground select-none group-data-[collapsible=icon]:hidden peer-hover/menu-button:text-sidebar-accent-foreground peer-hover/menu-button:[--show:1] hover:[--show:1] focus-within:[--show:1]',
        refreshing || turning ? '[--show:1]' : '[--show:0]',
        className,
      )}
    >
      <span className="pointer-events-none flex items-center px-1 opacity-[calc(1-var(--show))] transition-opacity motion-reduce:transition-none">
        {showCount && (
          <span aria-label={!countsKnown ? t('mail:unreadCountUnknown') : undefined}>{countsKnown ? unread : '—'}</span>
        )}
      </span>
      <Tooltip>
        <TooltipTrigger
          render={(
            <button
              type="button"
              aria-label={actionLabel}
              aria-disabled={busy}
              onClick={() => { if (!busy) refresh() }}
              className="absolute inset-0 flex items-center justify-center rounded-md opacity-(--show) outline-hidden transition-opacity motion-reduce:transition-none hover:bg-sidebar-accent hover:text-sidebar-accent-foreground focus-visible:ring-2 focus-visible:ring-sidebar-ring"
            />
          )}
        >
          {turning ? (
            <span role="status" aria-label={t('mail:syncing')}><SyncIcon className="size-3.5" spinning /></span>
          ) : <SyncIcon className="size-3.5" spinning={false} />}
        </TooltipTrigger>
        <TooltipContent side="right" className="flex-col items-start">
          <p>{actionLabel}</p>
          {sync?.lastSyncedAt && <p>{t('mail:lastSyncSucceededAt', { time: formatFullTime(sync.lastSyncedAt) })}</p>}
        </TooltipContent>
      </Tooltip>
    </div>
  )
}

type FolderItemProps = {
  node: FolderNode
  selectedId: string | null
  collapsed: Set<string>
  onToggle: (id: string, open: boolean) => void
  onSelect: (folder: Folder) => void
}

function FolderItem({ node, selectedId, collapsed, onToggle, onSelect }: FolderItemProps) {
  useTranslation()
  const { folder, children } = node
  const hasChildren = children.length > 0
  const open = hasChildren && !collapsed.has(folder.id)
  // A collapsed parent carries its children's unread mail so nothing is hidden,
  // except junk, which never adds to a number.
  const isJunk = folder.role === 'junk'
  const own = open || !hasChildren
  const unread = own ? countedUnread(folder) : node.unreadWithChildren
  const countsKnown = own ? !folder.selectable || folder.countsKnown : node.countsKnown
  // Junk shows a dot, not a number. A collapsed parent shows one when junk below it
  // has unread mail, so the signal is not hidden by folding the tree.
  const junkDot = own ? isJunk && folder.selectable && folder.unreadCount > 0 : node.junkUnread
  const label = (
    <>
      <HugeiconsIcon icon={ROLE_ICONS[folder.role]} />
      <span>{folderLabel(folder)}</span>
    </>
  )

  return (
    <Collapsible open={open} onOpenChange={(next) => onToggle(folder.id, next)} render={<SidebarMenuItem />}>
      {folder.selectable ? (
        <SidebarMenuButton
          isActive={folder.id === selectedId}
          onClick={() => onSelect(folder)}
          className={cn(hasChildren && unread > 0 && 'group-has-data-[sidebar=menu-action]/menu-item:pr-14')}
        >
          {label}
        </SidebarMenuButton>
      ) : (
        // IMAP \Noselect containers only group their children.
        <CollapsibleTrigger render={<SidebarMenuButton className="text-sidebar-foreground/70" />}>{label}</CollapsibleTrigger>
      )}
      {junkDot && countsKnown && unread === 0 ? (
        <SidebarMenuBadge aria-label={t('mail:hasUnread')} className={cn(hasChildren && 'right-7')}>
          <span className="size-1.5 rounded-full bg-current" />
        </SidebarMenuBadge>
      ) : (!countsKnown || unread > 0) && !(isJunk && own) && (
        <SidebarMenuBadge aria-label={!countsKnown ? t('mail:unreadCountUnknown') : undefined} className={cn(hasChildren && 'right-7')}>{countsKnown ? unread : '—'}</SidebarMenuBadge>
      )}
      {hasChildren && (
        <>
          <CollapsibleTrigger render={<SidebarMenuAction aria-label={t('mail:toggleFolder', { action: open ? t('mail:collapse') : t('mail:expand'), folder: folderLabel(folder) })} />}>
            <HugeiconsIcon icon={ArrowRight01Icon} className={cn('transition-transform', open && 'rotate-90')} />
          </CollapsibleTrigger>
          <CollapsibleContent>
            <SidebarMenuSub>
              {children.map((child) => (
                <FolderItem
                  key={child.folder.id}
                  node={child}
                  selectedId={selectedId}
                  collapsed={collapsed}
                  onToggle={onToggle}
                  onSelect={onSelect}
                />
              ))}
            </SidebarMenuSub>
          </CollapsibleContent>
        </>
      )}
    </Collapsible>
  )
}

function SyncFailureAction({ failure }: { failure: SyncFailure }) {
  useTranslation()
  return (
    <Tooltip>
      <TooltipTrigger render={<SidebarMenuAction aria-label={t('mail:syncFailure', { reason: errorText(failure) })} className="text-destructive" />}>
        <HugeiconsIcon icon={Alert02Icon} />
      </TooltipTrigger>
      <TooltipContent side="right" className="max-w-72">
        <p>{errorText(failure)}</p>
        {failure.detail && <p className="mt-1 opacity-70">{failure.detail}</p>}
        <p className="mt-1 opacity-70">{failure.willRetry ? t('mail:willRetryAutomatically') : failure.code === 'providerPolicy' ? t('mail:retryAfterProviderPolicy') : failure.code === 'signInRequired' ? t('mail:retryAfterSignIn') : t('mail:retryAfterAccountChange')}</p>
      </TooltipContent>
    </Tooltip>
  )
}
