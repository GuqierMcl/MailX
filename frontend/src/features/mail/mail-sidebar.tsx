import { useState } from 'react'
import { HugeiconsIcon, type IconSvgElement } from '@hugeicons/react'
import {
  Alert02Icon,
  Archive02Icon,
  ArrowRight01Icon,
  Delete02Icon,
  File01Icon,
  Folder01Icon,
  InboxIcon,
  PencilEdit02Icon,
  SentIcon,
  Settings01Icon,
  SpamIcon,
} from '@hugeicons/core-free-icons'
import type { Account, AccountSyncState, Folder, FolderRole, SyncFailure } from '@/generated/accounts'
import { Button } from '@/components/ui/button'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible'
import {
  Sidebar,
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
import { Spinner } from '@/components/ui/spinner'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'
import { buildFolderTree, loadCollapsed, saveCollapsed, type FolderNode } from './folder-tree'
import { folderLabel, type Mailbox } from './mailbox'

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
  onOpenSettings: () => void
}

export function MailSidebar({ accounts, folders, loading, syncStates, mailbox, onSelect, onOpenSettings }: MailSidebarProps) {
  const [collapsed, setCollapsed] = useState(loadCollapsed)
  const toggle = (id: string, open: boolean) => {
    const next = new Set(collapsed)
    if (open) next.delete(id)
    else next.add(id)
    setCollapsed(next)
    saveCollapsed(next)
  }

  const unifiedUnread = folders
    .filter((folder) => folder.role === 'inbox')
    .reduce((total, folder) => total + folder.unreadCount, 0)
  const selectedId = mailbox.kind === 'folder' ? mailbox.folder.id : null

  return (
    <Sidebar collapsible="offcanvas">
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
                {unifiedUnread > 0 && <SidebarMenuBadge>{unifiedUnread}</SidebarMenuBadge>}
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
                      render={<SidebarMenuButton title={account.emailAddress} className="font-medium" />}
                    >
                      <HugeiconsIcon icon={ArrowRight01Icon} className={cn('transition-transform', open && 'rotate-90')} />
                      <span>{account.displayName}</span>
                    </CollapsibleTrigger>
                    {unread > 0 && <SidebarMenuBadge className={cn(sync?.failure && 'right-7')}>{unread}</SidebarMenuBadge>}
                    {sync?.failure && <SyncFailureAction failure={sync.failure} />}
                    <CollapsibleContent>
                      <SidebarMenuSub>
                        {tree.length === 0 && (
                          <SidebarMenuItem>
                            <span className="flex h-8 items-center gap-2 px-2 text-xs text-sidebar-foreground/70">
                              {sync?.phase === 'failed' ? '暂无文件夹' : (
                                <>
                                  <Spinner className="size-3.5" aria-label="正在同步" />
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
            <SidebarMenuButton onClick={onOpenSettings}>
              <HugeiconsIcon icon={Settings01Icon} />
              <span>设置</span>
            </SidebarMenuButton>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarFooter>
    </Sidebar>
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
  const unread = open || !hasChildren ? folder.unreadCount : node.unreadWithChildren
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
      {unread > 0 && <SidebarMenuBadge className={cn(hasChildren && 'right-7')}>{unread}</SidebarMenuBadge>}
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
