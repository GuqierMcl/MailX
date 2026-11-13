import { errorText } from '@/i18n/errors'
import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { useCallback, useEffect, useMemo, useRef, useState, type RefObject } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import { Add01Icon, Mail01Icon } from '@hugeicons/core-free-icons'
import type { Account, AccountRemoval, AccountSyncState } from '@/generated/accounts'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Delayed } from '@/components/ui/delayed'
import { toast } from '@/components/ui/toast'
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '@/components/ui/dialog'
import { Empty, EmptyContent, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from '@/components/ui/empty'
import {
  Sidebar,
  SidebarContent,
  SidebarGroup,
  SidebarGroupContent,
  SidebarHeader,
  SidebarProvider,
} from '@/components/ui/sidebar'
import { Skeleton } from '@/components/ui/skeleton'
import { SettingsGroup, SettingsPage, SettingsRow } from '@/features/settings/settings-layout'
import { useSettings, useUpdateLayout } from '@/features/settings/settings-queries'
import { clientFailure, type ClientFailure, type MailxClient } from '@/lib/mailx-client'
import { sortAccountsForDisplay } from './account-order'
import { AccountHeader, type ProfilePreview } from './account-header'
import { accountIssues, connectionMethod, credentialProblem, firstProblemAccountId } from './account-status'
import { SortableAccountList } from './sortable-account-list'
import { ConnectionVerificationActions, VerificationHelp, VerificationNotice, VerificationSummary } from './connection-verification'
import { useConnectionVerification, verificationFailed, verificationReport } from './use-connection-verification'
import { ConnectionDetails } from './connection-details'
import { SignInAgain } from './sign-in-again'
import { ConnectionDialog } from './connection-dialog'
import { ProfileFields } from './profile-fields'
import { useAccount } from './use-account'
import { RemoveAccountDialog } from './remove-account-dialog'

/** Where the dialog opens; without `accountId` it selects the first account with a problem, else the first account. */
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
  const settings = useSettings(client)
  const order = useUpdateLayout(client)
  const accountOrder = settings.data?.settings.layout.accountOrder
  const orderedAccounts = useMemo(() => sortAccountsForDisplay(accounts, accountOrder ?? []), [accounts, accountOrder])

  // Reset the view whenever the dialog is opened with a new target.
  const [openedFor, setOpenedFor] = useState<AccountsTarget | null>(null)
  if (target !== openedFor) {
    setOpenedFor(target)
    if (target) setSelectedId(target.accountId ?? null)
  }

  // Without an explicit pick, open on the account that needs attention.
  const implicitId = useCallback((list: Account[]) => firstProblemAccountId(list, syncStates) ?? list[0]?.id ?? null, [syncStates])
  const visibleAccounts = orderedAccounts.filter((account) => !removedIds.has(account.id))
  const activeId = visibleAccounts.some((account) => account.id === selectedId) ? selectedId : implicitId(visibleAccounts)
  // Once loaded, pin the implicit first selection by identity so changing the
  // display order never replaces the profile being edited.
  if (target === openedFor && target && settings.data && activeId !== selectedId) setSelectedId(activeId)
  const removed = useCallback((result: AccountRemoval) => {
    setRemovedIds((previous) => new Set(previous).add(result.accountId))
    setRemovingId((current) => current === result.accountId ? null : current)
    setEditingConnection((current) => current === result.accountId ? null : current)
    setSelectedId((current) => {
      const effectiveId = current ?? implicitId(orderedAccounts.filter((account) => !removedIds.has(account.id)))
      if (effectiveId && effectiveId !== result.accountId) return effectiveId
      const index = orderedAccounts.findIndex((account) => account.id === result.accountId)
      return [...orderedAccounts.slice(index + 1), ...orderedAccounts.slice(0, index).reverse()]
        .find((account) => account.id !== result.accountId && !removedIds.has(account.id))?.id ?? null
    })
    onAccountsChanged()
  }, [orderedAccounts, removedIds, implicitId, onAccountsChanged])
  useEffect(() => client.onAccountRemoved(removed), [client, removed])

  return (
    <>
      <Dialog open={target !== null} onOpenChange={(open, details) => {
        // dnd-kit's keyboard sensor consumes the first Escape to cancel a drag.
        if (details.reason === 'escape-key' && details.event.defaultPrevented) return
        if (!open && !removing) onClose()
      }}>
        <DialogContent showCloseButton={!removing} className="flex h-[min(640px,calc(100dvh-4rem))] flex-col gap-0 overflow-hidden p-0 md:max-w-[760px] lg:max-w-[880px]">
          <DialogDescription className="sr-only">{t('accounts:accountsDescription')}</DialogDescription>
          {/* Narrow windows turn the account list into a top strip instead of hiding it. */}
          <SidebarProvider className="min-h-0 flex-1 flex-col md:flex-row">
            <Sidebar collapsible="none"
              className="h-auto w-full shrink-0 border-b pr-12 md:h-full md:w-(--sidebar-width) md:border-b-0 md:pr-0">
              <SidebarHeader className="shrink-0 flex-row items-center justify-between gap-3 px-4 py-3 md:pt-5">
                <DialogTitle className="min-w-0 text-base font-semibold">{t('accounts:emailAccounts')}</DialogTitle>
                <Button size="sm" disabled={removing} onClick={onAddAccount}>
                  <HugeiconsIcon icon={Add01Icon} aria-hidden />
                  {t('accounts:addAccountShort')}
                </Button>
              </SidebarHeader>
              <SidebarContent>
                <SidebarGroup className="py-2 md:p-2">
                  <SidebarGroupContent>
                    <SortableAccountList accounts={visibleAccounts} activeId={activeId} syncStates={syncStates} disabled={removing}
                      reorderDisabled={removing || order.isPending || !settings.data}
                      onSelect={setSelectedId} onReorder={(accountOrder) => order.mutateAsync({ accountOrder }).catch((cause: unknown) => {
                        toast.error(t('accounts:accountOrderNotSaved'), { description: errorText(cause) })
                        throw cause
                      })} />
                  </SidebarGroupContent>
                </SidebarGroup>
              </SidebarContent>
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
  const edits = useRef<RemovalBarrier>({ removed: false, wait: Promise.resolve() })
  useEffect(() => client.onAccountRemoved((result) => {
    if (result.accountId === accountId) {
      edits.current.removed = true
      edits.current.resume?.()
    }
  }), [client, accountId])

  // The remove dialog holds profile saves while it works, and drops them once the account is gone.
  const holdEdits = (busy: boolean) => {
    if (busy) edits.current.wait = new Promise((resolve) => { edits.current.resume = resolve })
    else { edits.current.resume?.(); edits.current.resume = undefined }
  }
  const dropEdits = () => {
    edits.current.removed = true
    edits.current.resume?.()
  }

  // With nothing to show, the failure stays on the page with its retry.
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
  return <LoadedAccount client={client} account={account} sync={sync} edits={edits} holdEdits={holdEdits} dropEdits={dropEdits} reload={reload} replace={replace}
    onSaved={onSaved} onEditConnection={onEditConnection} onRemoved={onRemoved} onRemovingChange={onRemovingChange} />
}

/**
 * One loaded account. States (what is wrong, what the connection is) live in
 * the page; events (a failed save, a sign-in) are toasts. The risky action sits
 * on the left of the fixed footer, the everyday ones on the right.
 */
function LoadedAccount({ client, account, sync, edits, holdEdits, dropEdits, reload, replace, onSaved, onEditConnection, onRemoved, onRemovingChange }: Omit<AccountDetailProps, 'accountId'> & {
  account: Account
  edits: RefObject<RemovalBarrier>
  holdEdits: (busy: boolean) => void
  dropEdits: () => void
  reload: () => Promise<void>
  replace: (account: Account) => void
}) {
  useTranslation()
  const [confirming, setConfirming] = useState(false)
  const [removing, setRemoving] = useState(false)
  const [preview, setPreview] = useState<ProfilePreview>(() => ({ displayName: account.displayName, color: account.color, useProviderIcon: account.useProviderIcon }))
  // Set by the user opening or folding the details; otherwise they open on a failed check.
  const [detailsOpen, setDetailsOpen] = useState<boolean | null>(null)
  const verification = useConnectionVerification(account, client)
  const issues = accountIssues(account, sync)
  const problem = credentialProblem(account)
  const passed = verificationReport(verification.state) !== null && !verificationFailed(verification.state)

  return (
    <SettingsPage footer={<>
      <Button variant="destructive" disabled={removing || !client.available} onClick={() => setConfirming(true)}>{t('accounts:removeAccount')}</Button>
      <div className="flex flex-wrap items-center justify-end gap-2">
        <ConnectionVerificationActions verification={verification} client={client} onStart={() => setDetailsOpen(null)} />
        {issues.oauth ? (
          <SignInAgain
            client={client}
            account={account}
            disabled={removing}
            required={issues.stoppedForSignIn}
            onSignedIn={(renewed) => {
              replace(renewed)
              onSaved()
            }}
          />
        ) : <Button variant="outline" disabled={removing} onClick={onEditConnection}>{t('accounts:editConnection')}</Button>}
      </div>
    </>}>
      <AccountHeader
        client={client}
        account={account}
        profile={{ ...preview, displayName: preview.displayName.trim() || account.displayName }}
        sync={sync}
        issues={issues}
      />
      <fieldset disabled={removing} className="contents"><ProfileEditor
        client={client}
        account={account}
        removal={edits}
        onPreview={(patch) => setPreview((current) => ({ ...current, ...patch }))}
        onSaved={(saved) => {
          replace(saved)
          onSaved()
        }}
      /></fieldset>
      <SettingsGroup title={t('accounts:connection')} description={t('accounts:connectionDescription')}>
        <SettingsRow label={t('accounts:connectionMethod')} description={t(passed ? 'accounts:verificationSucceeded' : 'accounts:verificationHint')}>
          <span className="flex flex-wrap items-baseline justify-end gap-x-1.5 text-xs">
            <span>{connectionMethod(account)}</span>
            {problem && <span className="text-destructive">· {problem}</span>}
            <VerificationSummary state={verification.state} />
          </span>
        </SettingsRow>
        <VerificationNotice state={verification.state} />
      </SettingsGroup>
      <ConnectionDetails account={account} verification={verification.state} open={detailsOpen ?? verificationFailed(verification.state)} onOpenChange={setDetailsOpen} />
      <VerificationHelp report={verificationReport(verification.state)} client={client} providerId={account.providerId} />
      {confirming && <RemoveAccountDialog
        key={`${account.id}:${account.revision}`}
        account={account} client={client}
        onClose={() => setConfirming(false)}
        onBusyChange={(busy) => {
          holdEdits(busy)
          setRemoving(busy)
          onRemovingChange(busy)
        }}
        onRemoved={(result) => { dropEdits(); onRemoved(result) }}
        onReload={() => { setConfirming(false); void reload() }}
      />}
    </SettingsPage>
  )
}

type ProfileEditorProps = {
  client: MailxClient
  account: Account
  onSaved: (account: Account) => void
  /** Follows the fields as they are edited, before anything is saved. */
  onPreview: (patch: Partial<ProfilePreview>) => void
  removal: RefObject<RemovalBarrier>
}

type RemovalBarrier = { removed: boolean; wait: Promise<void>; resume?: () => void }

type SaveStatus = { kind: 'idle' } | { kind: 'saving' } | { kind: 'saved' }

/**
 * Saves the name when editing it settles and the color on each pick, one
 * request at a time, always sending the latest committed profile. A name left
 * mid-edit is saved when the editor goes away.
 */
function ProfileEditor({ client, account, onSaved, onPreview, removal }: ProfileEditorProps) {
  useTranslation()
  const [displayName, setDisplayName] = useState(account.displayName)
  const [color, setColor] = useState(account.color)
  const [senderName, setSenderName] = useState(account.senderName ?? '')
  const [useProviderIcon, setUseProviderIcon] = useState(account.useProviderIcon)
  const [nameError, setNameError] = useState<ClientFailure | null>(null)
  const [senderNameError, setSenderNameError] = useState<ClientFailure | null>(null)
  const [status, setStatus] = useState<SaveStatus>({ kind: 'idle' })
  const committed = useRef({ displayName: account.displayName, color: account.color, senderName: account.senderName, useProviderIcon: account.useProviderIcon })
  const queue = useRef<Promise<void>>(Promise.resolve())
  const typed = useRef(displayName)
  const alive = useRef(true)

  const save = (patch: Partial<typeof committed.current>) => {
    if (removal.current.removed) return
    const profile = { ...committed.current, ...patch }
    if (profile.displayName === committed.current.displayName && profile.color === committed.current.color &&
      profile.senderName === committed.current.senderName && profile.useProviderIcon === committed.current.useProviderIcon) return
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
        // A save that fails is an event: say so even if the editor has gone, and leave the fields as they are.
        if (committed.current === profile) {
          toast.error(t('accounts:profileNotSaved'), { description: errorText(failure) })
          if (alive.current) setStatus({ kind: 'idle' })
        }
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
      <ProfileFields
        displayName={displayName}
        color={color}
        providerId={account.providerId}
        useProviderIcon={useProviderIcon}
        onUseProviderIconChange={(next) => {
          setUseProviderIcon(next)
          onPreview({ useProviderIcon: next })
          save({ useProviderIcon: next })
        }}
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
          onPreview({ displayName: value })
          setNameError(null)
        }}
        onDisplayNameCommit={commitName}
        onColorPreview={(next) => {
          setColor(next)
          onPreview({ color: next })
        }}
        onColorChange={(next) => {
          setColor(next)
          onPreview({ color: next })
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
