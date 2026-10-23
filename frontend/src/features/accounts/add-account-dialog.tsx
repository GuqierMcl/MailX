import { useState } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import { Tick02Icon } from '@hugeicons/core-free-icons'
import type { Account, ConnectionInput } from '@/generated/accounts'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogTitle } from '@/components/ui/dialog'
import { FieldDescription } from '@/components/ui/field'
import { ScrollArea } from '@/components/ui/scroll-area'
import { SettingsPage } from '@/features/settings/settings-layout'
import { clientFailure, type ClientFailure, type MailxClient } from '@/lib/mailx-client'
import { cn } from '@/lib/utils'
import { clearSecrets, type AccountDraft } from './account-draft'
import { nextAccountColor } from './account-color'
import { ConnectionForm } from './connection-form'
import { useLeaveGuard } from './leave-guard'
import { ProfileFields } from './profile-fields'
import { ProviderIcon } from './provider-badge'
import { PROVIDERS, type MailProvider } from './providers'

const STEP_COUNT = 3
/** Fields a failed create can blame on the profile step; anything else is the connection's. */
const PROFILE_FIELDS = new Set(['displayName', 'color', 'senderName'])

type Step =
  | { kind: 'method'; selected: MailProvider | null }
  | { kind: 'connection'; method: MailProvider; resume?: AccountDraft; error?: ClientFailure }
  | { kind: 'profile'; method: MailProvider; connection: ConnectionInput; draft: AccountDraft }

type AddAccountDialogProps = {
  client: MailxClient
  open: boolean
  /** Existing accounts, so the suggested color differs from theirs. */
  accounts: Account[]
  onClose: () => void
  /** Called once with the saved account; the caller decides where to go next. */
  onCreated: (account: Account) => void
}

/**
 * Adding an account as a short guided flow: pick a method, connect, then name
 * it. The connection step is the same form used to edit a connection later,
 * and the last step the same fields used to edit the profile.
 */
export function AddAccountDialog({ client, open, accounts, onClose, onCreated }: AddAccountDialogProps) {
  const guard = useLeaveGuard()
  const [step, setStep] = useState<Step>({ kind: 'method', selected: null })
  const [formVersion, setFormVersion] = useState(0)
  // Which way the last step change went, so the new step slides in from that side.
  const [direction, setDirection] = useState<'forward' | 'back'>('forward')
  const [displayName, setDisplayName] = useState('')
  const [nameTouched, setNameTouched] = useState(false)
  const [color, setColor] = useState<string | null>(null)
  const [senderName, setSenderName] = useState('')
  const [senderNameError, setSenderNameError] = useState<string | null>(null)
  const [nameError, setNameError] = useState<string | null>(null)
  const [saveError, setSaveError] = useState<ClientFailure | null>(null)
  const [saving, setSaving] = useState(false)
  // One request id per opening, so retrying a create that may have landed is safe.
  const [requestId, setRequestId] = useState('')

  // Every opening starts over.
  const [wasOpen, setWasOpen] = useState(false)
  if (open !== wasOpen) {
    setWasOpen(open)
    if (open) {
      setStep({ kind: 'method', selected: null })
      setDirection('forward')
      setDisplayName('')
      setNameTouched(false)
      setColor(nextAccountColor(accounts))
      setNameError(null)
      setSenderName('')
      setSenderNameError(null)
      setSaveError(null)
      guard.setDirty(false)
      setRequestId(crypto.randomUUID())
    }
  }

  // Anything typed past the method cards counts as unsaved.
  const unsaved = step.kind === 'profile' || guard.dirty
  const stepNumber = step.kind === 'method' ? 1 : step.kind === 'connection' ? 2 : 3

  const showConnection = (method: MailProvider, resume?: AccountDraft, error?: ClientFailure, way: 'forward' | 'back' = 'forward') => {
    setDirection(way)
    setStep({ kind: 'connection', method, resume, error })
    setFormVersion((version) => version + 1)
  }

  const back = () => {
    // The chosen method stays selected, so going back and forth is one click.
    if (step.kind === 'connection') {
      guard.leave(() => {
        setDirection('back')
        setStep({ kind: 'method', selected: step.method })
      })
    }
    // Going back from the last step keeps everything, passwords included.
    else if (step.kind === 'profile') showConnection(step.method, step.draft, undefined, 'back')
  }

  const finish = async () => {
    if (step.kind !== 'profile' || saving) return
    const name = displayName.trim()
    if (!name) {
      setNameError('请填写账号名称。')
      document.getElementById('displayName')?.focus()
      return
    }
    setSaving(true)
    guard.setBusy(true)
    setSaveError(null)
    try {
      const saved = await client.createAccount({ requestId, config: { ...step.connection, providerId: step.method.id, displayName: name, color, senderName: senderName.trim() || null } })
      guard.setDirty(false)
      onCreated(saved)
    } catch (cause) {
      const failure = clientFailure(cause)
      if (failure.field === 'displayName') setNameError(failure.message)
      else if (failure.field === 'senderName') setSenderNameError(failure.message)
      else if (failure.field && !PROFILE_FIELDS.has(failure.field)) {
        // The connection was rejected: go back to it with the error. The
        // passwords were in the failed request and must be entered again.
        showConnection(step.method, clearSecrets(step.draft), failure, 'back')
      } else setSaveError(failure)
    } finally {
      setSaving(false)
      guard.setBusy(false)
    }
  }

  return (
    <>
      <Dialog open={open} onOpenChange={(next) => !next && guard.leave(onClose, unsaved)}>
        <DialogContent className="flex h-[min(640px,calc(100dvh-4rem))] flex-col gap-0 overflow-hidden p-0 md:max-w-[640px]">
          <DialogDescription className="sr-only">选择添加邮箱账号的方式，连接邮箱，再为它命名。</DialogDescription>
          <header className="flex shrink-0 items-baseline gap-2 px-6 py-4 pr-12">
            <DialogTitle className="text-base font-semibold">添加邮箱账号</DialogTitle>
            {step.kind !== 'method' && <span className="text-sm text-muted-foreground">{step.method.name}</span>}
            <span className="ml-auto text-xs text-muted-foreground tabular-nums">第 {stepNumber} 步，共 {STEP_COUNT} 步</span>
          </header>
          {/* Keyed by step so each change replays the slide; skipped when reduced motion is preferred. */}
          <div key={step.kind} className={cn(
            'flex min-h-0 flex-1 flex-col duration-200 ease-out animate-in fade-in-0 motion-reduce:animate-none',
            direction === 'forward' ? 'slide-in-from-right-6' : 'slide-in-from-left-6',
          )}>
            {step.kind === 'method' && (
              <>
                <p id="add-account-methods" className="shrink-0 px-6 pb-3 text-sm text-muted-foreground">选择你的邮箱服务商。</p>
                {/* The list of providers will grow, so it scrolls inside a dialog of fixed height. */}
                <ScrollArea className="min-h-0 flex-1">
                  <div role="radiogroup" aria-labelledby="add-account-methods" className="grid grid-cols-2 gap-3 px-6 pb-6 sm:grid-cols-3">
                    {PROVIDERS.map((item) => {
                      const checked = step.selected === item
                      return (
                        <button key={item.id ?? 'generic'} type="button" role="radio" aria-checked={checked}
                          onClick={() => setStep({ kind: 'method', selected: item })}
                          onDoubleClick={() => showConnection(item)}
                          className={cn(
                            'relative flex flex-col items-center gap-2 rounded-lg border px-3 py-4 text-center outline-none transition-colors hover:bg-muted/50 focus-visible:ring-2 focus-visible:ring-ring',
                            checked && 'border-primary bg-primary/5 ring-1 ring-primary hover:bg-primary/5',
                          )}>
                          {checked && (
                            <span className="absolute top-2 right-2 flex size-4 items-center justify-center rounded-full bg-primary text-primary-foreground">
                              <HugeiconsIcon icon={Tick02Icon} className="size-3" strokeWidth={3} />
                            </span>
                          )}
                          <ProviderIcon provider={item} />
                          <span className="text-sm font-medium">{item.name}</span>
                          <span className="text-xs text-muted-foreground">{item.domain}</span>
                        </button>
                      )
                    })}
                  </div>
                </ScrollArea>
                <footer className="flex shrink-0 items-center justify-end gap-4 border-t px-6 py-3">
                  <Button disabled={!step.selected} onClick={() => step.selected && showConnection(step.selected)}>下一步</Button>
                </footer>
              </>
            )}
            {step.kind === 'connection' && (
              <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
                <ConnectionForm
                  key={formVersion}
                  account={null}
                  initialDraft={step.resume}
                  initialError={step.error}
                  intro={{ title: '手动配置 · 密码或授权码', description: '填写服务商提供的收发服务器参数。下一步为这个邮箱命名。' }}
                  submitLabel="下一步"
                  submittingLabel="下一步"
                  onBack={back}
                  onBusyChange={guard.setBusy}
                  onDirtyChange={(dirty) => dirty && guard.setDirty(true)}
                  onSubmit={(connection, draft) => {
                    if (!nameTouched) setDisplayName(connection.emailAddress)
                    setNameError(null)
                    setSaveError(null)
                    setDirection('forward')
                    setStep({ kind: 'profile', method: step.method, connection, draft })
                  }}
                />
              </div>
            )}
            {step.kind === 'profile' && (
              <div className="flex min-h-0 flex-1 flex-col overflow-y-auto">
                <SettingsPage footer={<>
                  <FieldDescription>{saving ? '正在安全保存配置…' : `邮箱：${step.connection.emailAddress}`}</FieldDescription>
                  <div className="flex shrink-0 gap-2">
                    <Button variant="outline" onClick={back} disabled={saving}>上一步</Button>
                    <Button onClick={() => void finish()} disabled={saving || !client.available}>{saving ? '保存中…' : '完成'}</Button>
                  </div>
                </>}>
                  <Alert role="status">
                    <AlertTitle>最后，给这个邮箱起个名字</AlertTitle>
                    <AlertDescription>名称和颜色只用于在 MailX 中区分邮箱；发件人名称是收件人看到的名字，可以留空。都可以之后修改。完成后 MailX 会登录收信服务器同步文件夹。</AlertDescription>
                  </Alert>
                  {saveError && (
                    <Alert variant="destructive">
                      <AlertTitle>尚未保存</AlertTitle>
                      <AlertDescription>{saveError.message}</AlertDescription>
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
                    disabled={saving}
                    onDisplayNameChange={(value) => {
                      setDisplayName(value)
                      setNameTouched(true)
                      setNameError(null)
                    }}
                    onColorChange={setColor}
                  />
                </SettingsPage>
              </div>
            )}
          </div>
        </DialogContent>
      </Dialog>
      {guard.confirm}
    </>
  )
}
