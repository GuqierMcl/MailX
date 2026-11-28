import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useQuery } from '@tanstack/react-query'
import { HugeiconsIcon } from '@hugeicons/react'
import { Delete02Icon, Edit02Icon } from '@hugeicons/core-free-icons'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import { ErrorNotice } from '@/components/error-notice'
import {
  AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent,
  AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { Delayed } from '@/components/ui/delayed'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Spinner } from '@/components/ui/spinner'
import { AccountAvatar } from '@/features/accounts/account-avatar'
import { sortAccountsForDisplay } from '@/features/accounts/account-order'
import { loadMailTree } from '@/features/mail/mail-tree'
import type { AccountSignatureMode, Signature } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'
import { mailKeys } from '@/lib/query-client'
import { SettingsGroup, SettingsLoadError, SettingsPage, SettingsRow } from './settings-layout'
import { useSettings } from './settings-queries'
import { SignatureDialog } from './signature-dialog'
import { useDeleteSignature, useSetAccountSignature, useSetDefaultSignature, useSignatures } from './signature-queries'

const FOLLOW = '__follow__'
const NONE = '__none__'

function signaturePreview(sig: Signature): string {
  const text = sig.bodyText.replace(/\s+/g, ' ').trim()
  if (text) return text
  if (sig.images.length > 0) return t('settings:signaturesImageOnly')
  return ''
}

export function SignaturesPane({ client }: { client: MailxClient }) {
  useTranslation('settings')
  const signatures = useSignatures(client)
  const settings = useSettings(client)
  const accounts = useQuery({ queryKey: mailKeys.tree(), queryFn: () => loadMailTree(client), select: (tree) => tree.accounts })
  const remove = useDeleteSignature(client)
  const setGlobalDefault = useSetDefaultSignature(client)
  const setAccountDefault = useSetAccountSignature(client)

  const [dialog, setDialog] = useState<{ kind: 'new' } | { kind: 'edit'; signature: Signature } | null>(null)
  const [removing, setRemoving] = useState<Signature | null>(null)
  const [listError, setListError] = useState<string | null>(null)
  const [defaultError, setDefaultError] = useState<string | null>(null)

  if (signatures.isPending) {
    return <Delayed><div className="flex justify-center p-10"><Spinner className="size-5" /></div></Delayed>
  }
  if (signatures.isError) {
    return (
      <SettingsLoadError
        message={t('settings:signaturesReadFailed')}
        error={signatures.error}
        onRetry={() => void signatures.refetch()}
        disabled={signatures.isFetching}
      />
    )
  }

  const state = signatures.data
  const globalDefault = state.signatures.find((sig) => sig.isDefault) ?? null
  const accountOrder = settings.data?.settings.layout.accountOrder ?? []
  const sortedAccounts = sortAccountsForDisplay(accounts.data ?? [], accountOrder)
  const overrideByAccount = new Map(state.accountDefaults.map((item) => [item.accountId, item.selection]))

  const followLabel = globalDefault
    ? t('settings:signaturesFollowDefaultNamed', { name: globalDefault.name })
    : t('settings:signaturesFollowDefault')
  const signatureItems: Record<string, string> = {
    [FOLLOW]: followLabel,
    [NONE]: t('settings:signaturesNone'),
    ...Object.fromEntries(state.signatures.map((sig) => [sig.id, sig.name])),
  }

  const addButton = (
    <Button variant="outline" size="sm" onClick={() => setDialog({ kind: 'new' })}>
      {t('settings:signaturesAdd')}
    </Button>
  )

  const removingInUse = removing !== null && (
    removing.isDefault ||
    state.accountDefaults.some((item) => item.selection.mode === 'specific' && item.selection.signatureId === removing.id)
  )

  const toggleDefault = (sig: Signature) => {
    setListError(null)
    void setGlobalDefault
      .mutateAsync({ signatureId: sig.isDefault ? null : sig.id })
      .catch((error: unknown) => setListError(errorText(error)))
  }

  const selectValueFor = (selection: AccountSignatureMode | undefined): string => {
    if (!selection || selection.mode === 'followDefault') return FOLLOW
    if (selection.mode === 'none') return NONE
    return state.signatures.some((s) => s.id === selection.signatureId) ? selection.signatureId : FOLLOW
  }

  return (
    <SettingsPage>
      <SettingsGroup title={t('settings:signaturesList')}>
        {state.signatures.length === 0 ? (
          <div className="flex flex-col items-start gap-3">
            <p className="text-sm text-muted-foreground">{t('settings:signaturesEmpty')}</p>
            {addButton}
          </div>
        ) : (
          <>
            <ul className="flex flex-col gap-1">
              {state.signatures.map((sig) => {
                const preview = signaturePreview(sig)
                return (
                  <li key={sig.id} className="group flex items-center gap-2 rounded-lg px-2 py-2 -mx-2 transition-colors hover:bg-foreground/5">
                    <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                      <div className="flex min-w-0 items-center gap-2">
                        <span className="truncate text-sm font-medium">{sig.name}</span>
                        {sig.isDefault && (
                          <Badge variant="secondary" className="h-5 shrink-0 px-1.5 text-[11px] font-normal">
                            {t('settings:signaturesDefaultBadge')}
                          </Badge>
                        )}
                      </div>
                      {preview && <span className="truncate text-xs text-muted-foreground">{preview}</span>}
                    </div>
                    <Button
                      variant="ghost"
                      size="xs"
                      className="shrink-0 text-muted-foreground opacity-0 transition-opacity hover:text-foreground group-hover:opacity-100 focus-visible:opacity-100"
                      disabled={setGlobalDefault.isPending}
                      onClick={() => toggleDefault(sig)}
                    >
                      {sig.isDefault ? t('settings:signaturesClearDefault') : t('settings:signaturesSetDefault')}
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      aria-label={t('settings:signaturesEdit', { name: sig.name })}
                      onClick={() => setDialog({ kind: 'edit', signature: sig })}
                    >
                      <HugeiconsIcon icon={Edit02Icon} strokeWidth={2} />
                    </Button>
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      aria-label={t('settings:signaturesDelete', { name: sig.name })}
                      onClick={() => setRemoving(sig)}
                    >
                      <HugeiconsIcon icon={Delete02Icon} strokeWidth={2} />
                    </Button>
                  </li>
                )
              })}
            </ul>
            <div>{addButton}</div>
          </>
        )}
        {listError && <ErrorNotice message={t('settings:saveFailedTitle')} detail={listError} />}
      </SettingsGroup>

      {state.signatures.length > 0 && sortedAccounts.length > 0 && (
        <SettingsGroup title={t('settings:signaturesDefaults')} description={t('settings:signaturesDefaultsDescription')}>
          {sortedAccounts.map((account) => {
            const value = selectValueFor(overrideByAccount.get(account.id))
            return (
              <SettingsRow
                key={account.id}
                htmlFor={`signature.default.${account.id}`}
                label={
                  <span className="flex min-w-0 items-center gap-2">
                    <AccountAvatar
                      displayName={account.displayName}
                      color={account.color}
                      providerId={account.providerId}
                      useProviderIcon={account.useProviderIcon}
                      className="size-5 shrink-0 text-[10px]"
                    />
                    <span className="truncate">{account.displayName}</span>
                  </span>
                }
                description={account.emailAddress}
              >
                <Select
                  value={value}
                  items={signatureItems}
                  onValueChange={(next: string | null) => {
                    if (!next) return
                    setDefaultError(null)
                    const selection: AccountSignatureMode = next === FOLLOW
                      ? { mode: 'followDefault' }
                      : next === NONE
                        ? { mode: 'none' }
                        : { mode: 'specific', signatureId: next }
                    void setAccountDefault
                      .mutateAsync({ accountId: account.id, selection })
                      .catch((error: unknown) => setDefaultError(errorText(error)))
                  }}
                >
                  <SelectTrigger id={`signature.default.${account.id}`} className="w-52">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    {Object.entries(signatureItems).map(([id, label]) => (
                      <SelectItem key={id} value={id}>{label}</SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </SettingsRow>
            )
          })}
          {defaultError && <ErrorNotice message={t('settings:saveFailedTitle')} detail={defaultError} />}
        </SettingsGroup>
      )}

      {dialog && (
        <SignatureDialog
          client={client}
          editing={dialog.kind === 'edit' ? dialog.signature : null}
          onClose={() => setDialog(null)}
        />
      )}

      <AlertDialog open={removing !== null} onOpenChange={(open) => !open && setRemoving(null)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t('settings:signaturesDeleteTitle', { name: removing?.name })}</AlertDialogTitle>
            <AlertDialogDescription>
              {removingInUse ? t('settings:signaturesDeleteInUse') : t('settings:signaturesDeleteBody')}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t('settings:signaturesCancel')}</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={() => {
                if (!removing) return
                const id = removing.id
                setRemoving(null)
                setListError(null)
                void remove.mutateAsync(id).catch((error: unknown) => setListError(errorText(error)))
              }}
            >
              {t('settings:signaturesDeleteConfirm')}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </SettingsPage>
  )
}
