import { useState } from 'react'
import { HugeiconsIcon, type IconSvgElement } from '@hugeicons/react'
import {
  Alert02Icon,
  Archive02Icon,
  ArrowRight01Icon,
  AtSignIcon,
  Delete02Icon,
  File01Icon,
  Folder01Icon,
  InboxIcon,
  PencilEdit02Icon,
  RefreshIcon,
  SentIcon,
  Settings01Icon,
  SpamIcon,
} from '@hugeicons/core-free-icons'
import type { Account, AccountSyncState, Folder, FolderRole, SyncFailure } from '@/generated/accounts'
import { Button } from '@/components/ui/button'
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
import { buildFolderTree, loadCollapsed, saveCollapsed, type FolderNode } from './folder-tree'
import { folderLabel, type Mailbox } from './mailbox'
import { AccountAvatar } from '@/features/accounts/account-avatar'

const ROLE_ICONS: Record<FolderRole, IconSvgElement> = {
  inbox: InboxIcon,
  drafts: File01Icon,
  sent: SentIcon,
  archive: Archive02Icon,
  junk: SpamIcon,
  trash: Delete02Icon,
  other: Folder01Icon,
}

type MailSidebarProps = {
  accounts: Account[]
  folders: Folder[]
  loading: boolean
  /** Latest sync state by account id; missing means no sync has reported yet. */
  syncStates: Record<string, AccountSyncState>
  mailbox: Mailbox
  onSelect: (mailbox: Mailbox) => void
  onOpenAccounts: () => void
  onOpenSettings: () => void
  /** Starts the account's next sync round now. */
  onRefresh: (accountId: string) => void
}

export function MailSidebar({ accounts, folders, loading, syncStates, mailbox, onSelect, onOpenAccounts, onOpenSettings, onRefresh }: MailSidebarProps) {
  const [collapsed, setCollapsed] = useState(loadCollapsed)
  const toggle = (id: string, open: boolean) => {
    const next = new Set(collapsed)
    if (open) next.delete(id)
    else next.add(id)
    setCollapsed(next)
    saveCollapsed(next)
  }

  const inboxes = folders.filter((folder) => folder.selectable && folder.role === 'inbox')
  const unifiedUnread = inboxes.reduce((total, folder) => total + folder.unreadCount, 0)
  const unifiedCountsKnown = inboxes.every((folder) => folder.countsKnown)
  const selectedId = mailbox.kind === 'folder' ? mailbox.folder.id : null
  const hasSyncFailure = accounts.some((account) => syncStates[account.id]?.failure)

  return (
    <div data-slot="mail-sidebar" className="flex h-full min-h-0 flex-col">
      <SidebarHeader>
        {/* Composing is not implemented yet. */}
        <Button className="w-full" disabled>
          <HugeiconsIcon icon={PencilEdit02Icon} />
          写邮件
        </Button>
      </SidebarHeader>
      <SidebarContent>
        <SidebarGroup>
          <SidebarGroupContent>
            <SidebarMenu>
              <SidebarMenuItem>
                <SidebarMenuButton
                  isActive={mailbox.kind === 'unified'}
                  onClick={() => onSelect({ kind: 'unified', role: 'inbox' })}
                >
                  <HugeiconsIcon icon={InboxIcon} />
                  <span>全部收件箱</span>
                </SidebarMenuButton>
                {!unifiedCountsKnown ? <SidebarMenuBadge aria-label="未读数待刷新">—</SidebarMenuBadge> : unifiedUnread > 0 && <SidebarMenuBadge>{unifiedUnread}</SidebarMenuBadge>}
              </SidebarMenuItem>
            </SidebarMenu>
          </SidebarGroupContent>
        </SidebarGroup>
        <SidebarGroup>
          <SidebarGroupLabel>邮箱</SidebarGroupLabel>
          <SidebarGroupContent>
            <SidebarMenu>
              {loading &&
                Array.from({ length: 4 }, (_, index) => (
                  <SidebarMenuItem key={index}>
                    <SidebarMenuSkeleton showIcon />
                  </SidebarMenuItem>
                ))}
              {accounts.map((account) => {
                const tree = buildFolderTree(folders.filter((folder) => folder.accountId === account.id))
                const unread = tree.reduce((total, node) => total + node.unreadWithChildren, 0)
                const countsKnown = tree.every((node) => node.countsKnown)
                const open = !collapsed.has(account.id)
                const sync = syncStates[account.id]
                return (
                  <Collapsible
                    key={account.id}
                    open={open}
                    onOpenChange={(next) => toggle(account.id, next)}
                    render={<SidebarMenuItem />}
                  >
                    <CollapsibleTrigger
                      render={<SidebarMenuButton title={account.emailAddress} className={cn('font-medium', sync?.failure ? 'pr-14' : 'pr-8')} />}
                    >
                      <HugeiconsIcon icon={ArrowRight01Icon} className={cn('transition-transform', open && 'rotate-90')} />
                      <AccountAvatar displayName={account.displayName} color={account.color} className="size-[18px] text-[10px]" />
                      <span>{account.displayName}</span>
                    </CollapsibleTrigger>
                    <AccountSyncSlot
                      unread={unread}
                      countsKnown={countsKnown}
                      syncing={sync?.phase === 'syncing'}
                      label={account.displayName}
                      onRefresh={() => onRefresh(account.id)}
                      className={cn(sync?.failure && 'right-7')}
                    />
                    {sync?.failure && <SyncFailureAction failure={sync.failure} />}
                    <CollapsibleContent>
                      <SidebarMenuSub>
                        {tree.length === 0 && (
                          <SidebarMenuItem>
                            <span className="flex h-8 items-center gap-2 px-2 text-xs text-sidebar-foreground/70">
                              {sync?.phase === 'failed' || sync?.phase === 'idle' ? '暂无文件夹' : (
                                <>
                                  <SyncIcon className="size-3.5" spinning />
                                  正在同步文件夹…
                                </>
                              )}
                            </span>
                          </SidebarMenuItem>
                        )}
                        {tree.map((node) => (
                          <FolderItem
                            key={node.folder.id}
                            node={node}
                            selectedId={selectedId}
                            collapsed={collapsed}
                            onToggle={toggle}
                            onSelect={(folder) => onSelect({ kind: 'folder', folder })}
                          />
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
              <span>账号</span>
            </SidebarMenuButton>
            {hasSyncFailure && <SidebarMenuBadge aria-label="有账号同步失败" className="text-destructive">!</SidebarMenuBadge>}
          </SidebarMenuItem>
          <SidebarMenuItem>
            <SidebarMenuButton onClick={onOpenSettings}>
              <HugeiconsIcon icon={Settings01Icon} />
              <span>设置</span>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarFooter>
    </div>
  )
}

/** The sync glyph; it turns while a round runs. */
function SyncIcon({ spinning, className }: { spinning: boolean; className?: string }) {
  return (
    <HugeiconsIcon
      icon={RefreshIcon}
      strokeWidth={2}
      className={cn(className, spinning && 'animate-spin [animation-duration:1.4s]')}
    />
  )
}

/**
 * Sync status and manual refresh belong to each account's top-level row only.
 * While the account syncs (after a short delay, so quick background checks
 * don't blink) a turning glyph replaces the unread count; otherwise hovering
 * or focusing this row turns the count into a refresh button. Folder rows
 * below never react. `--show` is set only by this row's own button or slot,
 * never by an ancestor, because the account's `li` contains its whole subtree.
 */
function AccountSyncSlot({ unread, countsKnown, syncing, label, onRefresh, className }: {
  unread: number
  countsKnown: boolean
  syncing: boolean
  label: string
  onRefresh: () => void
  className?: string
}) {
  const turning = useDelayedFlag(syncing, 400, 700)
  const showCount = !countsKnown || unread > 0
  const canRefresh = !syncing
  return (
    <div
      className={cn(
        'absolute top-1.5 right-1 flex h-5 min-w-5 items-center justify-center rounded-md px-1 text-xs font-medium tabular-nums text-sidebar-foreground select-none group-data-[collapsible=icon]:hidden [--show:0]',
        canRefresh && 'peer-hover/menu-button:text-sidebar-accent-foreground peer-hover/menu-button:[--show:1] hover:[--show:1] focus-within:[--show:1]',
        className,
      )}
    >
      <span className={cn('pointer-events-none flex items-center transition-opacity', canRefresh && 'opacity-[calc(1-var(--show))]')}>
        {turning ? (
          <span role="status" aria-label="正在同步"><SyncIcon className="size-3.5" spinning /></span>
        ) : showCount && (
          <span aria-label={!countsKnown ? '未读数待刷新' : undefined}>{countsKnown ? unread : '—'}</span>
        )}
      </span>
      {canRefresh && (
        <button
          type="button"
          aria-label={`立即同步 ${label}`}
          title={`立即同步 ${label}`}
          onClick={onRefresh}
          className="absolute inset-0 flex items-center justify-center rounded-md opacity-(--show) outline-hidden transition-opacity hover:bg-sidebar-accent hover:text-sidebar-accent-foreground focus-visible:ring-2 focus-visible:ring-sidebar-ring"
        >
          <SyncIcon className="size-3.5" spinning={false} />
        </button>
      )}
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
  const { folder, children } = node
  const hasChildren = children.length > 0
  const open = hasChildren && !collapsed.has(folder.id)
  // A collapsed parent carries its children's unread mail so nothing is hidden.
  const unread = open || !hasChildren ? folder.selectable ? folder.unreadCount : 0 : node.unreadWithChildren
  const countsKnown = open || !hasChildren ? !folder.selectable || folder.countsKnown : node.countsKnown
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
      {(!countsKnown || unread > 0) && <SidebarMenuBadge aria-label={!countsKnown ? '未读数待刷新' : undefined} className={cn(hasChildren && 'right-7')}>{countsKnown ? unread : '—'}</SidebarMenuBadge>}
      {hasChildren && (
        <>
          <CollapsibleTrigger render={<SidebarMenuAction aria-label={`${open ? '收起' : '展开'}${folderLabel(folder)}`} />}>
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
  return (
    <Tooltip>
      <TooltipTrigger render={<SidebarMenuAction aria-label={`同步失败：${failure.message}`} className="text-destructive" />}>
        <HugeiconsIcon icon={Alert02Icon} />
      </TooltipTrigger>
      <TooltipContent side="right" className="max-w-72">
        <p>{failure.message}</p>
        {failure.detail && <p className="mt-1 opacity-70">{failure.detail}</p>}
        <p className="mt-1 opacity-70">{failure.willRetry ? '稍后会自动重试。' : '修改账号设置后会重新尝试。'}</p>
      </TooltipContent>
    </Tooltip>
  )
}
