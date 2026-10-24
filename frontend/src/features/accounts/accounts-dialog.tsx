import { useEffect, useRef, useState } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import { Add01Icon, Mail01Icon } from '@hugeicons/core-free-icons'
import type { Account, AccountSyncState, RouteConfig } from '@/generated/accounts'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Delayed } from '@/components/ui/delayed'
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '@/components/ui/dialog'
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupContent,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarProvider,
} from '@/components/ui/sidebar'
import { Skeleton } from '@/components/ui/skeleton'
import { SettingsGroup, SettingsPage, SettingsRow } from '@/features/settings/settings-layout'
import { clientFailure, type ClientFailure, type MailxClient } from '@/lib/mailx-client'
import { AccountAvatar } from './account-avatar'
import { ConnectionDialog } from './connection-dialog'
import { ProfileFields } from './profile-fields'
import { ProviderBadge } from './provider-badge'
import { providerOf } from './providers'
import { useAccount } from './use-account'

/** Where the dialog opens; without `accountId` it selects the first account. */
export type AccountsTarget = { accountId?: string }

type AccountsDialogProps = {
  client: MailxClient
  /** `null` keeps the dialog closed. */
  target: AccountsTarget | null
  accounts: Account[]
  /** Latest sync state by account id, to surface a stopped sync. */
  syncStates: Record<string, AccountSyncState>
  onClose: () => void
  onAccountsChanged: () => void
  onAddAccount: () => void
}

/**
 * Account management: the account list on the left, the selected account on
 * the right. The profile saves as it is edited; the connection is shown as a
 * summary and changed in its own dialog.
 */
export function AccountsDialog({ client, target, accounts, syncStates, onClose, onAccountsChanged, onAddAccount }: AccountsDialogProps) {
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [editingConnection, setEditingConnection] = useState<string | null>(null)
  const [connectionVersion, setConnectionVersion] = useState(0)

  // Reset the view whenever the dialog is opened with a new target.
  const [openedFor, setOpenedFor] = useState<AccountsTarget | null>(null)
  if (target !== openedFor) {
    setOpenedFor(target)
    if (target) setSelectedId(target.accountId ?? null)
  }

  const activeId = selectedId ?? accounts[0]?.id ?? null

  return (
    <>
      <Dialog open={target !== null} onOpenChange={(open) => !open && onClose()}>
        <DialogContent className="flex h-[min(640px,calc(100dvh-4rem))] flex-col gap-0 overflow-hidden p-0 md:max-w-[760px] lg:max-w-[880px]">
          <DialogDescription className="sr-only">查看邮箱账号，修改名称、标识颜色与连接。</DialogDescription>
          {/* Narrow windows turn the account list into a top strip instead of hiding it. */}
          <SidebarProvider className="min-h-0 flex-1 flex-col md:flex-row">
            <Sidebar collapsible="none"
              className="h-auto w-full shrink-0 border-b pr-12 md:h-full md:w-(--sidebar-width) md:border-b-0 md:pr-0">
              <SidebarHeader className="shrink-0 px-4 py-3 md:pt-5 md:pb-1">
                <DialogTitle className="text-base font-semibold">邮箱账号</DialogTitle>
              </SidebarHeader>
              <SidebarContent>
                <SidebarGroup className="py-2 md:p-2">
                  <SidebarGroupContent>
                    <SidebarMenu className="flex-row overflow-x-auto md:flex-col md:overflow-visible" aria-label="邮箱账号列表">
                      {accounts.map((row) => (
                        <SidebarMenuItem key={row.id} className="shrink-0">
                          <SidebarMenuButton isActive={row.id === activeId} title={`${row.emailAddress} · ${providerOf(row.providerId).name}`} onClick={() => setSelectedId(row.id)}>
                            <AccountAvatar displayName={row.displayName} color={row.color} className="size-5 text-[10px]" />
                            <span>{row.displayName}</span>
                          </SidebarMenuButton>
                        </SidebarMenuItem>
                      ))}
                    </SidebarMenu>
                  </SidebarGroupContent>
                </SidebarGroup>
              </SidebarContent>
              <SidebarFooter className="hidden md:flex">
                <Button variant="outline" className="w-full" onClick={onAddAccount}>
                  <HugeiconsIcon icon={Add01Icon} />
                  添加账号
                </Button>
              </SidebarFooter>
            </Sidebar>
            <main className="flex min-h-0 flex-1 flex-col overflow-y-auto pt-4 md:pt-12">
              {activeId ? (
                <AccountDetail
                  key={`${activeId}:${connectionVersion}`}
                  client={client}
                  accountId={activeId}
                  sync={syncStates[activeId]}
                  onSaved={onAccountsChanged}
                  onEditConnection={() => setEditingConnection(activeId)}
                />
              ) : (
                <SettingsPage>
                  <Empty className="border">
                    <EmptyHeader>
                      <EmptyMedia variant="icon">
                        <HugeiconsIcon icon={Mail01Icon} />
                      </EmptyMedia>
                      <EmptyTitle>从添加一个邮箱开始</EmptyTitle>
                      <EmptyDescription>连接邮箱后为它命名。设置保存在本机，密码交由系统凭据库管理。</EmptyDescription>
                    </EmptyHeader>
                    <EmptyContent>
                      <Button onClick={onAddAccount}>
                        <HugeiconsIcon icon={Add01Icon} />
                        添加第一个账号
                      </Button>
                    </EmptyContent>
                  </Empty>
                </SettingsPage>
              )}
              {/* The sidebar footer is hidden on narrow windows; keep adding reachable there. */}
              {activeId && (
                <div className="px-6 pb-4 md:hidden">
                  <Button variant="outline" onClick={onAddAccount}>
                    <HugeiconsIcon icon={Add01Icon} />
                    添加账号
                  </Button>
                </div>
              )}
            </main>
          </SidebarProvider>
        </DialogContent>
      </Dialog>
      <ConnectionDialog
        client={client}
        accountId={editingConnection}
        onClose={() => setEditingConnection(null)}
        onSaved={() => {
          setEditingConnection(null)
          // Reload the detail so its summary shows what was saved.
          setConnectionVersion((version) => version + 1)
          onAccountsChanged()
        }}
      />
    </>
  )
}

type AccountDetailProps = {
  client: MailxClient
  accountId: string
  sync: AccountSyncState | undefined
  onSaved: () => void
  onEditConnection: () => void
}

/** Loads one account; remounted per account and after its connection changes. */
function AccountDetail({ client, accountId, sync, onSaved, onEditConnection }: AccountDetailProps) {
  const { account, error, reload, replace } = useAccount(client, accountId)

  if (error) {
    return (
      <SettingsPage>
        <Alert variant="destructive">
          <AlertTitle>无法加载账号配置</AlertTitle>
          <AlertDescription>{error.message}</AlertDescription>
          <Button variant="outline" className="mt-3 w-fit" onClick={() => void reload()}>重试加载</Button>
        </Alert>
      </SettingsPage>
    )
  }
  if (!account) {
    return (
      <SettingsPage>
        <Delayed>
          <div role="status" aria-label="正在加载账号配置" className="flex flex-col gap-3">
            <Skeleton className="h-16 w-full" />
            <Skeleton className="h-48 w-full" />
          </div>
        </Delayed>
      </SettingsPage>
    )
  }

  const needsTrust = account.authentication.imap.trust === 'needsConfirmation'
    || account.authentication.smtp.trust === 'needsConfirmation'
    || (account.route.kind === 'custom' && account.route.authentication?.trust === 'needsConfirmation')
  // A failure that will be retried is transient; only a stopped sync needs the user.
  const stopped = sync?.failure && !sync.failure.willRetry ? sync.failure : null

  return (
    <SettingsPage>
      {needsTrust ? (
        <Alert variant="destructive">
          <AlertTitle>凭证需要重新确认</AlertTitle>
          <AlertDescription>服务器或登录身份已改变，在重新输入密码之前 MailX 不会用旧凭证连接。</AlertDescription>
          <Button variant="outline" className="mt-3 w-fit" onClick={onEditConnection}>修改连接</Button>
        </Alert>
      ) : stopped && (
        <Alert variant="destructive">
          <AlertTitle>同步已停止</AlertTitle>
          <AlertDescription>{stopped.message} 修改连接后会重新尝试。</AlertDescription>
          <Button variant="outline" className="mt-3 w-fit" onClick={onEditConnection}>修改连接</Button>
        </Alert>
      )}
      <ProfileEditor
        client={client}
        account={account}
        onSaved={(saved) => {
          replace(saved)
          onSaved()
        }}
      />
      <SettingsGroup title="连接" badge={<ProviderBadge providerId={account.providerId} />} description="服务器、凭证与网络路由。修改后 MailX 会立即重新连接。">
        <SettingsRow label="邮箱地址" description={account.emailAddress} />
        <SettingsRow label="收信服务器" description={server('IMAP', account.imap)} />
        <SettingsRow label="发信服务器" description={server('SMTP', account.smtp)} />
        <SettingsRow label="凭证" description={credentials(account, needsTrust)} />
        <SettingsRow label="网络路由" description={route(account.route)} />
        <Button variant="outline" className="w-fit" onClick={onEditConnection}>修改连接</Button>
      </SettingsGroup>
    </SettingsPage>
  )
}

type ProfileEditorProps = {
  client: MailxClient
  account: Account
  onSaved: (account: Account) => void
}

type SaveStatus = { kind: 'idle' } | { kind: 'saving' } | { kind: 'saved' } | { kind: 'failed'; failure: ClientFailure }

/**
 * Saves the name when editing it settles and the color on each pick, one
 * request at a time, always sending the latest committed profile. A name left
 * mid-edit is saved when the editor goes away.
 */
function ProfileEditor({ client, account, onSaved }: ProfileEditorProps) {
  const [displayName, setDisplayName] = useState(account.displayName)
  const [color, setColor] = useState(account.color)
  const [senderName, setSenderName] = useState(account.senderName ?? '')
  const [nameError, setNameError] = useState<string | null>(null)
  const [senderNameError, setSenderNameError] = useState<string | null>(null)
  const [status, setStatus] = useState<SaveStatus>({ kind: 'idle' })
  const committed = useRef({ displayName: account.displayName, color: account.color, senderName: account.senderName })
  const queue = useRef<Promise<void>>(Promise.resolve())
  const typed = useRef(displayName)
  const alive = useRef(true)

  const save = (patch: Partial<typeof committed.current>) => {
    const profile = { ...committed.current, ...patch }
    if (profile.displayName === committed.current.displayName && profile.color === committed.current.color &&
      profile.senderName === committed.current.senderName) return
    committed.current = profile
    if (alive.current) setStatus({ kind: 'saving' })
    queue.current = queue.current.then(async () => {
      try {
        const saved = await client.updateProfile({ accountId: account.id, profile })
        // A newer commit is queued behind this one; let it report instead.
        if (committed.current !== profile) return
        onSaved(saved)
        if (alive.current) setStatus({ kind: 'saved' })
      } catch (cause) {
        const failure = clientFailure(cause)
        // A rejected sender name is shown on its field, not as a general failure.
        if (failure.field === 'senderName') {
          if (alive.current) {
            setSenderNameError(failure.message)
            setStatus({ kind: 'idle' })
          }
          committed.current = { ...committed.current, senderName: account.senderName }
          return
        }
        if (alive.current && committed.current === profile) setStatus({ kind: 'failed', failure })
      }
    })
  }
  // The latest `save` closure, for the unmount flush below.
  const saveRef = useRef(save)
  useEffect(() => {
    saveRef.current = save
  })

  const commitName = () => {
    const name = displayName.trim()
    if (!name) {
      setNameError('请填写账号名称。')
      return
    }
    setNameError(null)
    if (name !== displayName) setDisplayName(name)
    save({ displayName: name })
  }

  const commitSenderName = () => {
    const name = senderName.trim()
    if (name !== senderName) setSenderName(name)
    setSenderNameError(null)
    save({ senderName: name || null })
  }

  useEffect(() => {
    alive.current = true
    return () => {
      alive.current = false
      // Closing the dialog or switching accounts must not drop a typed name.
      const name = typed.current.trim()
      if (name) saveRef.current({ displayName: name })
    }
  }, [])

  return (
    <SettingsGroup
      title="资料"
      badge={<SaveBadge status={status} />}
      description="名称、发件人名称和标识颜色修改后自动保存，不会重新连接。">
      {status.kind === 'failed' && (
        <Alert variant="destructive">
          <AlertTitle>资料未保存</AlertTitle>
          <AlertDescription>{status.failure.message}</AlertDescription>
        </Alert>
      )}
      <ProfileFields
        displayName={displayName}
        color={color}
        nameError={nameError}
        senderName={senderName}
        senderNameError={senderNameError}
        onSenderNameChange={(value) => {
          setSenderName(value)
          setSenderNameError(null)
        }}
        onSenderNameCommit={commitSenderName}
        onDisplayNameChange={(value) => {
          typed.current = value
          setDisplayName(value)
          setNameError(null)
        }}
        onDisplayNameCommit={commitName}
        onColorChange={(next) => {
          setColor(next)
          save({ color: next })
        }}
      />
    </SettingsGroup>
  )
}

function SaveBadge({ status }: { status: SaveStatus }) {
  const text = status.kind === 'saving' ? '正在保存…' : status.kind === 'saved' ? '已保存' : null
  return <span role="status" className="text-xs font-normal text-muted-foreground">{text}</span>
}

function server(protocol: string, connection: Account['imap']) {
  const security = connection.tls === 'implicit' ? 'TLS' : 'STARTTLS'
  return `${protocol} · ${connection.host}:${connection.port} · ${security} · ${connection.username}`
}

function credentials(account: Account, needsTrust: boolean) {
  const mode = account.authentication.mode === 'shared' ? '收发使用相同密码' : '收发分别设置密码'
  const configured = account.authentication.imap.passwordConfigured && account.authentication.smtp.passwordConfigured
  return `${mode} · ${needsTrust ? '需要重新确认' : configured ? '已保存在系统凭据库' : '缺少密码'}`
}

function route(value: RouteConfig) {
  if (value.kind === 'direct') return '直接连接'
  if (value.kind === 'inherit') return '使用应用默认（目前为直接连接）'
  const kind = value.proxyKind === 'socks5' ? 'SOCKS5' : 'HTTP CONNECT'
  return `${kind} 代理 · ${value.host}:${value.port}${value.authentication ? ` · ${value.authentication.username}` : ''}`
}
