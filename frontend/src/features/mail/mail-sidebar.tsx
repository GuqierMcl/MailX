import { useState } from 'react'
import { HugeiconsIcon, type IconSvgElement } from '@hugeicons/react'
import {
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
import type { Account, Folder, FolderRole } from '@/generated/accounts'
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
import { cn } from '@/lib/utils'
import { buildFolderTree, loadCollapsed, saveCollapsed, type FolderNode } from './folder-tree'
import type { Mailbox } from './mailbox'

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
  mailbox: Mailbox
  onSelect: (mailbox: Mailbox) => void
  onOpenSettings: () => void
}

export function MailSidebar({ accounts, folders, loading, mailbox, onSelect, onOpenSettings }: MailSidebarProps) {
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
                    {unread > 0 && <SidebarMenuBadge>{unread}</SidebarMenuBadge>}
                    <CollapsibleContent>
                      <SidebarMenuSub>
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
      <span>{folder.name}</span>
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
          <CollapsibleTrigger render={<SidebarMenuAction aria-label={`${open ? '收起' : '展开'}${folder.name}`} />}>
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
