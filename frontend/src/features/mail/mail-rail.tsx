import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { HugeiconsIcon, type IconSvgElement } from '@hugeicons/react'
import {
  Archive03Icon,
  AtSignIcon,
  Delete02Icon,
  File01Icon,
  InboxIcon,
  MailUpload01Icon,
  MoreHorizontalIcon,
  PencilEdit02Icon,
  SentIcon,
  Settings01Icon,
  SpamIcon,
  StarIcon,
  SunriseIcon,
} from '@hugeicons/core-free-icons'
import type { FolderRole } from '@/generated/accounts'
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'
import { AccountAvatar } from '@/features/accounts/account-avatar'
import { buildFolderTree, type FolderNode } from './folder-tree'
import { folderLabel, type Mailbox } from './mailbox'
import { COMPOSE_SHORTCUT, ROLE_ICONS, type MailSidebarProps } from './sidebar-shared'

/** A small number on an icon; past 99 it stops growing. */
function RailCount({ count }: { count: number }) {
  return (
    <span
      aria-hidden
      className="pointer-events-none absolute -top-1 -right-1 flex h-4 min-w-4 items-center justify-center rounded-full bg-primary px-1 text-[10px] leading-none font-medium tabular-nums text-primary-foreground ring-2 ring-sidebar"
    >
      {count > 99 ? '99+' : count}
    </span>
  )
}

/** A dot on an icon, for junk or an account with unread mail; red for a failed sync. */
function RailDot({ alert = false }: { alert?: boolean }) {
  return (
    <span
      aria-hidden
      className={cn('pointer-events-none absolute top-0 right-0 size-2 rounded-full ring-2 ring-sidebar', alert ? 'bg-destructive' : 'bg-primary')}
    />
  )
}

const railBase = 'relative flex size-8 items-center justify-center rounded-[calc(var(--radius-sm)+2px)] outline-hidden ring-sidebar-ring focus-visible:ring-2 [&_svg]:size-4 [&_svg]:shrink-0'

/**
 * A row's own state decides its colours; hover adds only to the idle one. Mixing
 * both sets on one element did not work: a hover colour compiles to a more
 * specific selector than a `data-active` one, so hovering the selected view
 * turned its icon dark on its dark fill.
 */
const railIdle = 'text-sidebar-foreground hover:bg-sidebar-accent hover:text-sidebar-accent-foreground data-popup-open:bg-sidebar-accent data-popup-open:text-sidebar-accent-foreground'
/** The view on screen: a filled square that stays filled while hovered. */
const railChosen = 'bg-primary text-primary-foreground hover:bg-primary/90 data-popup-open:bg-primary/90'
const railButton = (active: boolean) => cn(railBase, active ? railChosen : railIdle)

/**
 * One rail icon with its name in a tooltip. The accessible name carries the
 * count or dot the icon shows, since neither is read out on its own.
 */
function RailItem({ icon, label, name, active = false, onClick, mark }: {
  icon: IconSvgElement
  label: string
  /** Accessible name, when it says more than `label` (e.g. the unread count). */
  name?: string
  active?: boolean
  onClick: () => void
  mark?: React.ReactNode
}) {
  return (
    <Tooltip>
      <TooltipTrigger
        render={<button type="button" aria-label={name ?? label} data-active={active || undefined} aria-current={active ? 'page' : undefined} className={railButton(active)} onClick={onClick} />}
      >
        <HugeiconsIcon icon={icon} />
        {mark}
      </TooltipTrigger>
      <TooltipContent side="right">{name ?? label}</TooltipContent>
    </Tooltip>
  )
}

/** Name of a row with its unread number, for the tooltip and screen readers. */
const withUnread = (label: string, count: number) => (count > 0 ? t('mail:railUnread', { label, count }) : label)

/** Folder rows of an account's menu, depth first, each indented under its parent. */
function flatten(nodes: FolderNode[], depth = 0): { node: FolderNode; depth: number }[] {
  return nodes.flatMap((node) => [{ node, depth }, ...flatten(node.children, depth + 1)])
}

/**
 * The folded sidebar: the same places as `MailSidebar` in one column of icons.
 * 更多 and each account open a menu rather than a tree, so the rail stays one
 * column wide.
 */
export function MailRail({ accounts, folders, syncStates, mailbox, onSelect, onOpenAccounts, onOpenSettings, onCompose, draftCount, outboxCount }: Omit<MailSidebarProps, 'loading' | 'onRefresh'>) {
  useTranslation()
  const inboxes = folders.filter((folder) => folder.selectable && folder.role === 'inbox')
  const unifiedUnread = inboxes.reduce((total, folder) => total + folder.unreadCount, 0)
  const archivedUnread = inboxes.reduce((total, folder) => total + folder.archivedUnreadCount, 0)
  const junkUnread = folders.some((folder) => folder.selectable && folder.role === 'junk' && folder.countsKnown && folder.unreadCount > 0)
  const hasSyncFailure = accounts.some((account) => syncStates[account.id]?.failure)
  const isUnified = (role: FolderRole) => mailbox.kind === 'unified' && mailbox.role === role
  const moreActive = isUnified('junk') || isUnified('trash') || (mailbox.kind === 'archived' && !mailbox.accountId)
  const choose = (next: Mailbox) => () => onSelect(next)

  return (
    <div data-slot="mail-rail" className="flex h-full min-h-0 flex-col items-center gap-1 py-2">
      <RailItem icon={PencilEdit02Icon} label={t('mail:composeShortcut', { shortcut: COMPOSE_SHORTCUT })} onClick={onCompose} />
      <div className="mt-3 flex flex-col items-center gap-1">
        <RailItem icon={SunriseIcon} label={t('mail:nowView')} active={mailbox.kind === 'now'} onClick={choose({ kind: 'now' })} />
        <RailItem
          icon={InboxIcon} label={t('mail:inbox')} name={withUnread(t('mail:inbox'), unifiedUnread)}
          active={isUnified('inbox')} onClick={choose({ kind: 'unified', role: 'inbox' })}
          mark={unifiedUnread > 0 && <RailCount count={unifiedUnread} />}
        />
        <RailItem icon={StarIcon} label={t('mail:starredView')} active={mailbox.kind === 'starred'} onClick={choose({ kind: 'starred' })} />
        {/* No number on the icon: drafts pile up and never clear themselves, so a
            standing badge turns into noise. The count is still in the tooltip. */}
        <RailItem
          icon={File01Icon} label={t('mail:drafts')} name={draftCount > 0 ? `${t('mail:drafts')} · ${t('mail:draftCount', { count: draftCount })}` : undefined}
          active={mailbox.kind === 'drafts'} onClick={choose({ kind: 'drafts' })}
        />
        <RailItem icon={SentIcon} label={t('mail:sent')} active={isUnified('sent')} onClick={choose({ kind: 'unified', role: 'sent' })} />
        {outboxCount > 0 && (
          <RailItem
            icon={MailUpload01Icon} label={t('mail:outbox')} name={`${t('mail:outbox')} · ${t('mail:outboxCount', { count: outboxCount })}`}
            active={mailbox.kind === 'outbox'} onClick={choose({ kind: 'outbox' })}
            mark={<RailCount count={outboxCount} />}
          />
        )}
        <DropdownMenu>
          <Tooltip>
            <TooltipTrigger
              render={<DropdownMenuTrigger render={<button type="button" aria-label={t('mail:more')} data-active={moreActive || undefined} className={railButton(moreActive)} />} />}
            >
              <HugeiconsIcon icon={MoreHorizontalIcon} />
            </TooltipTrigger>
            <TooltipContent side="right">{t('mail:more')}</TooltipContent>
          </Tooltip>
          <DropdownMenuContent side="right" align="start" className="w-44">
            <MenuRow icon={Archive03Icon} label={t('mail:archive')} count={archivedUnread} active={mailbox.kind === 'archived' && !mailbox.accountId} onClick={choose({ kind: 'archived', accountId: null })} />
            <MenuRow icon={SpamIcon} label={t('mail:junk')} dot={junkUnread} active={isUnified('junk')} onClick={choose({ kind: 'unified', role: 'junk' })} />
            <MenuRow icon={Delete02Icon} label={t('mail:trash')} active={isUnified('trash')} onClick={choose({ kind: 'unified', role: 'trash' })} />
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
      {/* Accounts scroll on their own, so compose and the footer stay put. */}
      <div className="mt-3 flex min-h-0 flex-1 flex-col items-center gap-1 overflow-y-auto py-1 [scrollbar-width:none]">
        {accounts.map((account) => {
          const tree = buildFolderTree(folders.filter((folder) => folder.accountId === account.id))
          const unread = tree.reduce((total, node) => total + node.unreadWithChildren, 0)
          const failure = Boolean(syncStates[account.id]?.failure)
          const active = (mailbox.kind === 'folder' && mailbox.folder.accountId === account.id) || (mailbox.kind === 'archived' && mailbox.accountId === account.id)
          const name = failure ? `${account.displayName} · ${t('mail:mailSyncFailed')}` : withUnread(account.displayName, unread)
          return (
            <DropdownMenu key={account.id}>
              <Tooltip>
                <TooltipTrigger
                  render={(
                    <DropdownMenuTrigger
                      // An avatar is not a glyph: the account in use stays on the light tint.
                      render={<button type="button" aria-label={name} data-active={active || undefined} className={cn(railBase, active ? 'bg-sidebar-accent hover:bg-sidebar-accent data-popup-open:bg-sidebar-accent' : railIdle)} />}
                    />
                  )}
                >
                  <AccountAvatar displayName={account.displayName} color={account.color} providerId={account.providerId} useProviderIcon={account.useProviderIcon} className="size-6 text-[11px]" />
                  {(failure || unread > 0) && <RailDot alert={failure} />}
                </TooltipTrigger>
                <TooltipContent side="right">{name}</TooltipContent>
              </Tooltip>
              <DropdownMenuContent side="right" align="start" className="max-h-[min(28rem,var(--available-height))] w-56">
                <DropdownMenuGroup>
                  <DropdownMenuLabel className="truncate" title={account.emailAddress}>{account.displayName}</DropdownMenuLabel>
                  {tree.length === 0 && <div className="px-2 py-1.5 text-xs text-muted-foreground">{t('mail:noFolders')}</div>}
                  {flatten(tree).flatMap(({ node, depth }) => {
                    const { folder } = node
                    const row = (
                      <MenuRow
                        key={folder.id}
                        icon={ROLE_ICONS[folder.role]}
                        label={folderLabel(folder)}
                        depth={depth}
                        disabled={!folder.selectable}
                        count={folder.role === 'junk' ? 0 : folder.selectable ? folder.unreadCount : 0}
                        dot={folder.role === 'junk' && folder.selectable && folder.unreadCount > 0}
                        active={mailbox.kind === 'folder' && mailbox.folder.id === folder.id}
                        onClick={choose({ kind: 'folder', folder })}
                      />
                    )
                    if (folder.role !== 'inbox' || !folder.selectable) return [row]
                    // Locally archived inbox mail, right after the inbox as in the full sidebar.
                    return [row, (
                      <MenuRow
                        key={`${folder.id}:archived`}
                        icon={Archive03Icon}
                        label={t('mail:archivedFolder')}
                        depth={depth}
                        count={folder.archivedUnreadCount}
                        active={mailbox.kind === 'archived' && mailbox.accountId === account.id}
                        onClick={choose({ kind: 'archived', accountId: account.id })}
                      />
                    )]
                  })}
                </DropdownMenuGroup>
              </DropdownMenuContent>
            </DropdownMenu>
          )
        })}
      </div>
      <div className="flex flex-col items-center gap-1 pt-1">
        <RailItem
          icon={AtSignIcon} label={t('mail:accounts')} name={hasSyncFailure ? `${t('mail:accounts')} · ${t('mail:someAccountsSyncFailed')}` : undefined}
          onClick={onOpenAccounts} mark={hasSyncFailure && <RailDot alert />}
        />
        <RailItem icon={Settings01Icon} label={t('mail:settings')} onClick={onOpenSettings} />
      </div>
    </div>
  )
}

/** A row in the 更多 or account menu: icon, name, and the same number or dot as the full sidebar. */
function MenuRow({ icon, label, count = 0, dot = false, depth = 0, active, disabled = false, onClick }: {
  icon: IconSvgElement
  label: string
  count?: number
  dot?: boolean
  depth?: number
  active: boolean
  disabled?: boolean
  onClick: () => void
}) {
  return (
    <DropdownMenuItem
      disabled={disabled}
      onClick={onClick}
      aria-current={active ? 'page' : undefined}
      style={depth > 0 ? { paddingLeft: `calc(0.5rem + ${depth}rem)` } : undefined}
      className={cn(active && 'font-medium text-primary')}
    >
      <HugeiconsIcon icon={icon} />
      <span className="flex-1 truncate">{label}</span>
      {count > 0 && <span className="text-xs tabular-nums text-muted-foreground">{count}</span>}
      {dot && <span aria-label={t('mail:hasUnread')} className="size-1.5 rounded-full bg-current" />}
    </DropdownMenuItem>
  )
}
