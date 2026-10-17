import { HugeiconsIcon, type IconSvgElement } from '@hugeicons/react'
import {
  Archive02Icon,
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
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuBadge,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSkeleton,
} from '@/components/ui/sidebar'
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
  const unifiedUnread = folders
    .filter((folder) => folder.role === 'inbox')
    .reduce((total, folder) => total + folder.unreadCount, 0)

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
        {loading && (
          <SidebarGroup>
            <SidebarMenu>
              {Array.from({ length: 4 }, (_, index) => (
                <SidebarMenuItem key={index}>
                  <SidebarMenuSkeleton showIcon />
                </SidebarMenuItem>
              ))}
            </SidebarMenu>
          </SidebarGroup>
        )}
        {accounts.map((account) => (
          <SidebarGroup key={account.id}>
            <SidebarGroupLabel title={account.emailAddress}>{account.displayName}</SidebarGroupLabel>
            <SidebarGroupContent>
              <SidebarMenu>
                {folders
                  .filter((folder) => folder.accountId === account.id)
                  .map((folder) => (
                    <SidebarMenuItem key={folder.id}>
                      <SidebarMenuButton
                        isActive={mailbox.kind === 'folder' && mailbox.folder.id === folder.id}
                        onClick={() => onSelect({ kind: 'folder', folder })}
                      >
                        <HugeiconsIcon icon={ROLE_ICONS[folder.role]} />
                        <span>{folder.name}</span>
                      </SidebarMenuButton>
                      {folder.unreadCount > 0 && <SidebarMenuBadge>{folder.unreadCount}</SidebarMenuBadge>}
                    </SidebarMenuItem>
                  ))}
              </SidebarMenu>
            </SidebarGroupContent>
          </SidebarGroup>
        ))}
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
