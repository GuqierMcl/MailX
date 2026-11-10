import { errorText } from '@/i18n/errors'
import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { useEffect, useRef, useState } from 'react'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { FieldDescription, FieldSet } from '@/components/ui/field'
import { SettingsGroup, SettingsPage } from '@/features/settings/settings-layout'
import { clientFailure, type ClientFailure, type MailxClient } from '@/lib/mailx-client'
import { changeDraft, draftToRoute, initialDraft, type AccountDraft } from './account-draft'
import { RouteFields, TextField } from './connection-form'
import type { MailProvider } from './providers'

type SignInStepProps = {
  client: MailxClient
  /** A provider that signs in in the browser (`signIn` is set). */
  provider: MailProvider
  /** Network settings to come back to, e.g. after the last step sent the user back here. */
  initialDraft?: AccountDraft
  /** Why the user is back here, e.g. the signed-in session ran out. */
  initialError?: ClientFailure | null
  /** The browser returned and the mail server opened the mailbox; the session stays alive for naming. */
  onSignedIn(result: { sessionId: string; emailAddress: string; draft: AccountDraft }): void
  onBack(): void
  /** Busy only while a quick check runs, so the dialog can still be closed while the browser is open. */
  onBusyChange(busy: boolean): void
  onDirtyChange(dirty: boolean): void
}

type Phase =
  | { kind: 'idle' }
  | { kind: 'waiting' }
  /** Signed in, but no mailbox yet: ask for the address. */
  | { kind: 'needsMailbox'; sessionId: string }
  | { kind: 'checking'; sessionId: string }

/**
 * Step 2 for browser sign-in providers. There is no address to type first: the
 * user signs in, and MailX learns the address from the sign-in. Only when the
 * provider's mail server won't open that mailbox does it ask for the address,
 * and then it reuses the sign-in it already has instead of opening the browser
 * again.
 */
export function SignInStep({ client, provider, initialDraft: resume, initialError = null, onSignedIn, onBack, onBusyChange, onDirtyChange }: SignInStepProps) {
  useTranslation()
  const setup = provider.signIn
  const [draft, setDraft] = useState(() => resume ?? initialDraft(null, provider))
  const [phase, setPhase] = useState<Phase>({ kind: 'idle' })
  const [address, setAddress] = useState('')
  const [error, setError] = useState<ClientFailure | null>(initialError)
  const [advanced, setAdvanced] = useState(!!resume)
  // The live host session, and whether the user already gave up on it.
  const session = useRef<{ id: string | null; canceled: boolean } | null>(null)
  const alive = useRef(true)
  // A session handed to the next step must outlive this component.
  const handedOff = useRef(false)

  function abandon() {
    const current = session.current
    session.current = null
    if (!current) return
    current.canceled = true
    if (current.id) void client.cancelOAuthAccount(current.id).catch(() => undefined)
  }

  useEffect(() => {
    alive.current = true
    return () => {
      alive.current = false
      onBusyChange(false)
      if (!handedOff.current) abandon()
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- only the unmount matters
  }, [])

  function change<K extends keyof AccountDraft>(key: K, value: AccountDraft[K]) {
    setDraft((current) => changeDraft(current, key, value))
    onDirtyChange(true)
    setError(null)
  }

  function finished(sessionId: string, emailAddress: string) {
    handedOff.current = true
    session.current = null
    onBusyChange(false)
    onSignedIn({ sessionId, emailAddress, draft })
  }

  function fail(cause: unknown) {
    const failure = clientFailure(cause)
    session.current = null
    onBusyChange(false)
    setPhase({ kind: 'idle' })
    setError(failure)
    if (failure.field) requestAnimationFrame(() => document.getElementById(failure.field!)?.focus())
  }

  async function start() {
    if (phase.kind !== 'idle') return
    setError(null)
    let route
    try {
      route = draftToRoute(draft)
    } catch (cause) {
      setAdvanced(true)
      fail(cause)
      return
    }
    const mine = { id: null as string | null, canceled: false }
    session.current = mine
    setPhase({ kind: 'waiting' })
    try {
      const handle = await client.beginOAuthAccount({ providerId: provider.id ?? '', route })
      mine.id = handle.sessionId
      // Canceled while the page was still opening.
      if (mine.canceled) {
        await client.cancelOAuthAccount(handle.sessionId).catch(() => undefined)
        return
      }
      const result = await client.finishOAuthAccount(handle.sessionId)
      if (!alive.current || mine.canceled) return
      if (result.kind === 'verified') finished(handle.sessionId, result.emailAddress)
      else {
        setAddress(result.suggestion ?? '')
        setPhase({ kind: 'needsMailbox', sessionId: handle.sessionId })
      }
    } catch (cause) {
      // The user canceled: stay here without an error.
      if (alive.current && !mine.canceled) fail(cause)
    }
  }

  async function confirm() {
    if (phase.kind !== 'needsMailbox') return
    const { sessionId } = phase
    setError(null)
    setPhase({ kind: 'checking', sessionId })
    onBusyChange(true)
    try {
      const result = await client.confirmOAuthMailbox({ sessionId, emailAddress: address.trim() })
      if (!alive.current) return
      if (result.kind === 'verified') finished(sessionId, result.emailAddress)
      else {
        setAddress(result.suggestion ?? address)
        onBusyChange(false)
        setPhase({ kind: 'needsMailbox', sessionId })
      }
    } catch (cause) {
      if (!alive.current) return
      const failure = clientFailure(cause)
      onBusyChange(false)
      // A refused address can be fixed and tried again; anything else ended the session.
      if (failure.code === 'oauthMailboxRefused') {
        setError(failure)
        setPhase({ kind: 'needsMailbox', sessionId })
        requestAnimationFrame(() => document.getElementById('emailAddress')?.focus())
      } else fail(cause)
    }
  }

  function cancel() {
    abandon()
    setPhase({ kind: 'idle' })
  }

  const waiting = phase.kind === 'waiting'
  const askingAddress = phase.kind === 'needsMailbox' || phase.kind === 'checking'
  const checking = phase.kind === 'checking'
  const addressError = askingAddress && error?.code === 'oauthMailboxRefused' ? { ...error, field: 'emailAddress' } as ClientFailure : null

  return (
    <SettingsPage footer={waiting ? <>
      <FieldDescription>{t('accounts:browserSignInNote')}</FieldDescription>
      <Button variant="outline" onClick={cancel}>{t('accounts:cancelSignIn')}</Button>
    </> : askingAddress ? <>
      <FieldDescription>{t('accounts:browserSignInNote')}</FieldDescription>
      <div className="flex shrink-0 gap-2">
        <Button variant="outline" onClick={() => { abandon(); onBack() }} disabled={checking}>{t('accounts:back')}</Button>
        <Button onClick={() => void confirm()} disabled={checking || !address.trim()}>{checking ? t('accounts:checkingAddress') : t('accounts:next')}</Button>
      </div>
    </> : <>
      <FieldDescription>{t('accounts:browserSignInNote')}</FieldDescription>
      <div className="flex shrink-0 gap-2">
        <Button variant="outline" onClick={onBack}>{t('accounts:back')}</Button>
        <Button onClick={() => void start()} disabled={!client.available}>{t('accounts:signInInBrowser')}</Button>
      </div>
    </>}>
      <Alert role="status">
        <AlertTitle>{setup?.title}</AlertTitle>
        <AlertDescription>{setup?.description}</AlertDescription>
      </Alert>
      {waiting && (
        <Alert role="status">
          <AlertTitle>{t('accounts:signInWaitingTitle')}</AlertTitle>
          <AlertDescription>{t('accounts:signInWaitingDescription')}</AlertDescription>
        </Alert>
      )}
      {error && !addressError && (
        <Alert variant="destructive">
          <AlertTitle>{t('accounts:signInFailed')}</AlertTitle>
          <AlertDescription>{errorText(error)}</AlertDescription>
        </Alert>
      )}
      {askingAddress && (
        <FieldSet disabled={checking} aria-busy={checking} className="gap-6">
          <Alert role="status">
            <AlertDescription>{t('accounts:signInNeedsAddress')}</AlertDescription>
          </Alert>
          <SettingsGroup title={t('accounts:emailAddress')} description={setup?.emailHint}>
            <TextField id="emailAddress" label={t('accounts:emailAddress')} value={address}
              onChange={(value) => { setAddress(value); setError(null) }}
              placeholder={setup?.emailPlaceholder} inputMode="email" error={addressError} />
          </SettingsGroup>
        </FieldSet>
      )}
      {phase.kind === 'idle' && (
        <FieldSet aria-label={t('accounts:networkRoute')} className="gap-6">
          <details open={advanced} onToggle={(event) => setAdvanced(event.currentTarget.open)}>
            <summary className="cursor-pointer rounded-md py-2 text-sm font-medium focus-visible:outline-2 focus-visible:outline-ring">{t('accounts:advancedSettings')}</summary>
            <div className="flex flex-col gap-6 pt-4">
              <RouteFields draft={draft} change={change} error={error} oldProxyAuth={null} />
            </div>
          </details>
        </FieldSet>
      )}
    </SettingsPage>
  )
}
