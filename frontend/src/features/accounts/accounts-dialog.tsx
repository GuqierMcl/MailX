import { errorText } from '@/i18n/errors'
import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { useCallback, useEffect, useRef, useState, type RefObject } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import { Add01Icon, Mail01Icon } from '@hugeicons/core-free-icons'
import type { Account, AccountRemoval, AccountSyncState, RouteConfig } from '@/generated/accounts'
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
import { ConnectionVerification } from './connection-verification'
import { QqAuthHelp } from './qq-auth-help'
import { ConnectionDialog } from './connection-dialog'
import { ProfileFields } from './profile-fields'
import { ProviderBadge } from './provider-badge'
import { providerOf } from './providers'
import { useAccount } from './use-account'
import { RemoveAccountDialog } from './remove-account-dialog'

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
  useTranslation()
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [editingConnection, setEditingConnection] = useState<string | null>(null)
  const [connectionVersion, setConnectionVersion] = useState(0)
  const [removedIds, setRemovedIds] = useState<Set<string>>(() => new Set())
  const [removingId, setRemovingId] = useState<string | null>(null)
  const removing = removingId !== null

  // Reset the view whenever the dialog is opened with a new target.
  const [openedFor, setOpenedFor] = useState<AccountsTarget | null>(null)
  if (target !== openedFor) {
    setOpenedFor(target)
    if (target) setSelectedId(target.accountId ?? null)
  }

  const visibleAccounts = accounts.filter((account) => !removedIds.has(account.id))
  const activeId = visibleAccounts.some((account) => account.id === selectedId) ? selectedId : visibleAccounts[0]?.id ?? null
  const removed = useCallback((result: AccountRemoval) => {
    setRemovedIds((previous) => new Set(previous).add(result.accountId))
    setRemovingId((current) => current === result.accountId ? null : current)
    setEditingConnection((current) => current === result.accountId ? null : current)
    setSelectedId((current) => {
      const effectiveId = current ?? accounts.find((account) => !removedIds.has(account.id))?.id ?? null
      if (effectiveId && effectiveId !== result.accountId) return effectiveId
      const index = accounts.findIndex((account) => account.id === result.accountId)
      return [...accounts.slice(index + 1), ...accounts.slice(0, index).reverse()]
        .find((account) => account.id !== result.accountId && !removedIds.has(account.id))?.id ?? null
    })
    onAccountsChanged()
  }, [accounts, removedIds, onAccountsChanged])
  useEffect(() => client.onAccountRemoved(removed), [client, removed])

  return (
    <>
      <Dialog open={target !== null} onOpenChange={(open) => !open && !removing && onClose()}>
        <DialogContent showCloseButton={!removing} className="flex h-[min(640px,calc(100dvh-4rem))] flex-col gap-0 overflow-hidden p-0 md:max-w-[760px] lg:max-w-[880px]">
          <DialogDescription className="sr-only">{t('accounts:accountsDescription')}</DialogDescription>
          {/* Narrow windows turn the account list into a top strip instead of hiding it. */}
          <SidebarProvider className="min-h-0 flex-1 flex-col md:flex-row">
            <Sidebar collapsible="none"
              className="h-auto w-full shrink-0 border-b pr-12 md:h-full md:w-(--sidebar-width) md:border-b-0 md:pr-0">
              <SidebarHeader className="shrink-0 px-4 py-3 md:pt-5 md:pb-1">
                <DialogTitle className="text-base font-semibold">{t('accounts:emailAccounts')}</DialogTitle>
              </SidebarHeader>
              <SidebarContent>
                <SidebarGroup className="py-2 md:p-2">
                  <SidebarGroupContent>
                    <SidebarMenu className="flex-row overflow-x-auto md:flex-col md:overflow-visible" aria-label={t('accounts:emailAccountList')}>
                      {visibleAccounts.map((row) => (
                        <SidebarMenuItem key={row.id} className="shrink-0">
                          <SidebarMenuButton disabled={removing} isActive={row.id === activeId} title={`${row.emailAddress} · ${providerOf(row.providerId).name}`} onClick={() => setSelectedId(row.id)}>
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
                <Button variant="outline" className="w-full" disabled={removing} onClick={onAddAccount}>
                  <HugeiconsIcon icon={Add01Icon} />

                  {t('accounts:addAccountShort')}
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
                  onRemoved={removed}
                  onRemovingChange={(busy) => setRemovingId((current) => busy ? activeId : current === activeId ? null : current)}
                />
              ) : (
                <SettingsPage>
                  <Empty className="border">
                    <EmptyHeader>
                      <EmptyMedia variant="icon">
                        <HugeiconsIcon icon={Mail01Icon} />
                      </EmptyMedia>
                      <EmptyTitle>{t('accounts:startWithAccount')}</EmptyTitle>
                      <EmptyDescription>{t('accounts:accountStorageHint')}</EmptyDescription>
                    </EmptyHeader>
                    <EmptyContent>
                      <Button onClick={onAddAccount}>
                        <HugeiconsIcon icon={Add01Icon} />

                        {t('accounts:addFirstAccount')}
                      </Button>
                    </EmptyContent>
                  </Empty>
                </SettingsPage>
              )}
              {/* The sidebar footer is hidden on narrow windows; keep adding reachable there. */}
              {activeId && (
                <div className="px-6 pb-4 md:hidden">
                  <Button variant="outline" disabled={removing} onClick={onAddAccount}>
                    <HugeiconsIcon icon={Add01Icon} />

                    {t('accounts:addAccountShort')}
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
  onRemoved: (result: AccountRemoval) => void
  onRemovingChange: (busy: boolean) => void
}

/** Loads one account; remounted per account and after its connection changes. */
function AccountDetail({ client, accountId, sync, onSaved, onEditConnection, onRemoved, onRemovingChange }: AccountDetailProps) {
  useTranslation()
  const { account, error, reload, replace } = useAccount(client, accountId)
  const [confirming, setConfirming] = useState(false)
  const [removing, setRemoving] = useState(false)
  const edits = useRef<RemovalBarrier>({ removed: false, wait: Promise.resolve() })
  useEffect(() => client.onAccountRemoved((result) => {
    if (result.accountId === accountId) {
      edits.current.removed = true
      edits.current.resume?.()
    }
  }), [client, accountId])

  if (error) {
    return (
      <SettingsPage>
        <Alert variant="destructive">
          <AlertTitle>{t('accounts:loadAccountFailed')}</AlertTitle>
          <AlertDescription>{errorText(error)}</AlertDescription>
          <Button variant="outline" className="mt-3 w-fit" onClick={() => void reload()}>{t('accounts:retryLoad')}</Button>
        </Alert>
      </SettingsPage>
    )
  }
  if (!account) {
    return (
      <SettingsPage>
        <Delayed>
          <div role="status" aria-label={t('accounts:loadingAccount')} className="flex flex-col gap-3">
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
          <AlertTitle>{t('accounts:credentialsNeedConfirmation')}</AlertTitle>
          <AlertDescription>{t('accounts:changedIdentityHint')}</AlertDescription>
          <Button variant="outline" className="mt-3 w-fit" onClick={onEditConnection}>{t('accounts:editConnection')}</Button>
        </Alert>
      ) : stopped && (
        <Alert variant="destructive">
          <AlertTitle>{t('accounts:syncStopped')}</AlertTitle>
          <AlertDescription>{errorText(stopped)}  {t('accounts:retryAfterConnectionChange')}</AlertDescription>
          {account.providerId === 'qq' && stopped.code === 'authentication' && <QqAuthHelp client={client} />}
          <Button variant="outline" className="mt-3 w-fit" onClick={onEditConnection}>{t('accounts:editConnection')}</Button>
        </Alert>
      )}
      <fieldset disabled={removing} className="contents"><ProfileEditor
        client={client}
        account={account}
        removal={edits}
        onSaved={(saved) => {
          replace(saved)
          onSaved()
        }}
      /></fieldset>
      <SettingsGroup title={t('accounts:connection')} badge={<ProviderBadge providerId={account.providerId} />} description={t('accounts:connectionDescription')}>
        <SettingsRow label={t('accounts:emailAddress')} description={account.emailAddress} />
        <SettingsRow label={t('accounts:incomingServer')} description={server('IMAP', account.imap)} />
        <SettingsRow label={t('accounts:outgoingServer')} description={server('SMTP', account.smtp)} />
        <SettingsRow label={t('accounts:credentials')} description={credentials(account, needsTrust)} />
        <SettingsRow label={t('accounts:networkRoute')} description={route(account.route)} />
        <div className="flex flex-wrap gap-2">
          <Button variant="outline" disabled={removing} onClick={onEditConnection}>{t('accounts:editConnection')}</Button>
          <Button variant="destructive" disabled={removing || !client.available} onClick={() => setConfirming(true)}>{t('accounts:removeAccount')}</Button>
        </div>
        <ConnectionVerification account={account} client={client} />
      </SettingsGroup>
      {confirming && <RemoveAccountDialog
        key={`${account.id}:${account.revision}`}
        account={account} client={client}
        onClose={() => setConfirming(false)}
        onBusyChange={(busy) => {
          if (busy) edits.current.wait = new Promise((resolve) => { edits.current.resume = resolve })
          else { edits.current.resume?.(); edits.current.resume = undefined }
          setRemoving(busy)
          onRemovingChange(busy)
        }}
        onRemoved={(result) => { edits.current.removed = true; edits.current.resume?.(); onRemoved(result) }}
        onReload={() => { setConfirming(false); void reload() }}
      />}
    </SettingsPage>
  )
}

type ProfileEditorProps = {
  client: MailxClient
  account: Account
  onSaved: (account: Account) => void
  removal: RefObject<RemovalBarrier>
}

type RemovalBarrier = { removed: boolean; wait: Promise<void>; resume?: () => void }

type SaveStatus = { kind: 'idle' } | { kind: 'saving' } | { kind: 'saved' } | { kind: 'failed'; failure: ClientFailure }

/**
 * Saves the name when editing it settles and the color on each pick, one
 * request at a time, always sending the latest committed profile. A name left
 * mid-edit is saved when the editor goes away.
 */
function ProfileEditor({ client, account, onSaved, removal }: ProfileEditorProps) {
  useTranslation()
  const [displayName, setDisplayName] = useState(account.displayName)
  const [color, setColor] = useState(account.color)
  const [senderName, setSenderName] = useState(account.senderName ?? '')
  const [nameError, setNameError] = useState<ClientFailure | null>(null)
  const [senderNameError, setSenderNameError] = useState<ClientFailure | null>(null)
  const [status, setStatus] = useState<SaveStatus>({ kind: 'idle' })
  const committed = useRef({ displayName: account.displayName, color: account.color, senderName: account.senderName })
  const queue = useRef<Promise<void>>(Promise.resolve())
  const typed = useRef(displayName)
  const alive = useRef(true)

  const save = (patch: Partial<typeof committed.current>) => {
    if (removal.current.removed) return
    const profile = { ...committed.current, ...patch }
    if (profile.displayName === committed.current.displayName && profile.color === committed.current.color &&
      profile.senderName === committed.current.senderName) return
    committed.current = profile
    if (alive.current) setStatus({ kind: 'saving' })
    queue.current = queue.current.then(async () => {
      await removal.current.wait
      if (removal.current.removed) return
      try {
        const saved = await client.updateProfile({ accountId: account.id, profile })
        await removal.current.wait
        if (removal.current.removed) return
        // A newer commit is queued behind this one; let it report instead.
        if (committed.current !== profile) return
        if (alive.current) onSaved(saved)
        if (alive.current) setStatus({ kind: 'saved' })
      } catch (cause) {
        await removal.current.wait
        if (removal.current.removed) return
        const failure = clientFailure(cause)
        // A rejected sender name is shown on its field, not as a general failure.
        if (failure.field === 'senderName') {
          if (alive.current) {
            setSenderNameError(failure)
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
      setNameError({ code: 'validation', field: 'displayName', message: '' })
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
      title={t('accounts:profile')}
      badge={<SaveBadge status={status} />}
      description={t('accounts:profileDescription')}>
      {status.kind === 'failed' && (
        <Alert variant="destructive">
          <AlertTitle>{t('accounts:profileNotSaved')}</AlertTitle>
          <AlertDescription>{errorText(status.failure)}</AlertDescription>
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
  useTranslation()
  const text = status.kind === 'saving' ? t('accounts:savingStatus') : status.kind === 'saved' ? t('accounts:saved') : null
  return <span role="status" className="text-xs font-normal text-muted-foreground">{text}</span>
}

function server(protocol: string, connection: Account['imap']) {
  const security = connection.tls === 'implicit' ? 'TLS' : 'STARTTLS'
  return `${protocol} · ${connection.host}:${connection.port} · ${security} · ${connection.username}`
}

function credentials(account: Account, needsTrust: boolean) {
  const mode = account.authentication.mode === 'shared' ? t('accounts:sharedPassword') : t('accounts:separatePasswords')
  const configured = account.authentication.imap.passwordConfigured && account.authentication.smtp.passwordConfigured
  return `${mode} · ${needsTrust ? t('accounts:needsConfirmation') : configured ? t('accounts:storedInCredentialStore') : t('accounts:missingPassword')}`
}

function route(value: RouteConfig) {
  if (value.kind === 'direct') return t('accounts:directConnection')
  if (value.kind === 'inherit') return t('accounts:appDefaultDirect')
  const kind = value.proxyKind === 'socks5' ? 'SOCKS5' : 'HTTP CONNECT'
  return t('accounts:proxySummary', { kind, host: value.host, port: value.port, authentication: value.authentication ? ` · ${value.authentication.username}` : '' })
}
