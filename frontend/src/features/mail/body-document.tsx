import { errorText } from '@/i18n/errors'
import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { lazy, Suspense, useEffect, useRef, useState, type ReactNode } from 'react'
import { convertFileSrc, isTauri } from '@tauri-apps/api/core'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Delayed } from '@/components/ui/delayed'
import { Skeleton } from '@/components/ui/skeleton'
import type { ImageFetchError, RemoteImagePolicy, RemoteImageStatus } from '@/generated/accounts'
import { clientFailure, type ClientFailure, type MailxClient } from '@/lib/mailx-client'
import { useUpdateSecurity } from '@/features/settings/settings-queries'
import { cn } from '@/lib/utils'
import { useBodyEnvironment } from './use-body-environment'

const DevelopmentBody = import.meta.env.DEV && !isTauri()
  ? lazy(() => import('./development-body')) : null

function bodyDocumentUrl(emailId: string, documentToken: string, dark: boolean) {
  const url = new URL(convertFileSrc('', 'mailx-body'))
  url.pathname = `/document/${encodeURIComponent(emailId)}`
  url.searchParams.set('token', documentToken)
  if (dark) url.searchParams.set('scheme', 'dark')
  return url.href
}

/** The translated variant of the body, which exists only once a translation has been made. */
export type BodyTranslation = {
  /** Target language the document is prepared for. */
  target: string
  /** Changes whenever a new translation replaces the old one. */
  version: number
  /** Whether the translation, rather than the original, is the one on screen. */
  showing: boolean
  /** The translated document could not be prepared. */
  onFailed: (failure: unknown) => void
}

export function BodyDocument({ client, emailId, hasRemoteImages = false, banner, translated = null }: {
  client: MailxClient; emailId: string; hasRemoteImages?: boolean
  /** A notice shown above the body's own notices, inside the same column. */
  banner?: ReactNode
  translated?: BodyTranslation | null
}) {
  const { visible, settings, dark } = useBodyEnvironment(client)
  const [version, setVersion] = useState(0)
  if (!visible) return null
  if (settings.isPending) return <BodySkeleton />
  // A settings read failure never silently grants remote access.
  const policy = settings.data?.settings.security.remoteImages ?? 'ask'
  return <Reader key={`${emailId}:${version}:${policy}:${dark}`} client={client} emailId={emailId} policy={policy} dark={dark}
    hasRemoteImages={hasRemoteImages} banner={banner} translated={translated} onReload={() => setVersion((value) => value + 1)} />
}

function Reader({ client, emailId, policy, dark, hasRemoteImages, banner, translated, onReload }: {
  client: MailxClient; emailId: string; policy: RemoteImagePolicy; dark: boolean; hasRemoteImages: boolean; banner: ReactNode
  translated: BodyTranslation | null; onReload: () => void
}) {
  useTranslation()
  const [granted, setGranted] = useState(false)
  const [dismissed, setDismissed] = useState(false)
  const [remember, setRemember] = useState(false)
  const update = useUpdateSecurity(client)
  // The translated document is prepared the first time it is shown, then kept alongside the original.
  const translationKey = translated ? `${translated.target}:${translated.version}` : null
  const [mountedKey, setMountedKey] = useState<string | null>(null)
  const [failedKey, setFailedKey] = useState<string | null>(null)
  if (translated?.showing && mountedKey !== translationKey) setMountedKey(translationKey)
  const translationMounted = translated !== null && mountedKey === translationKey && failedKey !== translationKey
  const showTranslated = translationMounted && translated.showing
  const allowImages = policy === 'automatic' || (policy === 'ask' && granted)
  const load = () => {
    if (remember) update.mutate({ remoteImages: 'automatic' })
    else setGranted(true)
  }
  return <div className="flex min-h-0 flex-1 flex-col gap-2">
    {banner}
    {policy === 'ask' && hasRemoteImages && !granted && !dismissed &&
      <div role="status" aria-label={t('mail:remoteImagesNotice')} className="flex h-9 w-full shrink-0 items-center gap-3 rounded-md bg-muted px-3 text-xs">
        <span className="min-w-0 flex-1 truncate" title={t('mail:containsRemoteImages')}>{t('mail:containsRemoteImages')}</span>
        <label className="flex shrink-0 cursor-pointer items-center gap-1.5"><Checkbox checked={remember} onCheckedChange={(value) => setRemember(value === true)} disabled={update.isPending} />{t('mail:doNotAskAgain')}</label>
        <Button size="sm" variant="outline" className="h-6 shrink-0 text-xs" onClick={load} disabled={update.isPending}>{update.isPending ? t('mail:savingEllipsis') : t('mail:loadImages')}</Button>
        <Button size="icon-sm" variant="ghost" className="size-6 shrink-0" aria-label={t('mail:closeRemoteImagesNotice')} onClick={() => setDismissed(true)} disabled={update.isPending}>×</Button>
      </div>}
    {update.isError && <Alert variant="destructive"><AlertTitle>{t('mail:settingsSaveFailed')}</AlertTitle><AlertDescription>{errorText(update.error)}</AlertDescription></Alert>}
    {/* Both documents stay mounted and share one cell, so each keeps its scroll position and nothing reloads on a switch. */}
    <div className="grid min-h-0 flex-1 grid-rows-[minmax(0,1fr)]">
      <div className={cn('col-start-1 row-start-1 flex min-h-0 flex-col', showTranslated && 'invisible')} inert={showTranslated}>
        {DevelopmentBody ? <Suspense fallback={<BodySkeleton />}><DevelopmentBody /></Suspense> :
          <BodySession key={String(allowImages)} client={client} emailId={emailId} allowImages={allowImages} hasRemoteImages={hasRemoteImages} dark={dark} onReload={onReload} />}
      </div>
      {translationMounted && <div className={cn('col-start-1 row-start-1 flex min-h-0 flex-col', !showTranslated && 'invisible')} inert={!showTranslated}>
        {DevelopmentBody ? <Suspense fallback={<BodySkeleton />}><DevelopmentBody translated /></Suspense> :
          <BodySession key={`${allowImages}:${translationKey}`} client={client} emailId={emailId} allowImages={allowImages} hasRemoteImages={hasRemoteImages}
            dark={dark} translation={translated.target} title={t('mail:htmlBodyTranslated')} onReload={onReload}
            onPrepareFailed={(failure) => { setFailedKey(translationKey); translated.onFailed(failure) }} />}
      </div>}
    </div>
  </div>
}

/** The host binds this read-only session to either a saved draft revision or an email. */
export function BodySession({ client, emailId, allowImages = false, hasRemoteImages = false, dark, draftRevision, translation, title, onPrepareFailed, onReload }: {
  client: MailxClient; emailId: string; allowImages?: boolean; hasRemoteImages?: boolean; dark: boolean; draftRevision?: number
  /** Target language of the translated variant of the email body; the original when absent. */
  translation?: string
  title?: string
  /** Called when the document could not be prepared (the failure also shows in place). */
  onPrepareFailed?: (failure: ClientFailure) => void
  onReload: () => void
}) {
  useTranslation()
  const [documentToken, setDocumentToken] = useState<string | null>(null)
  const [error, setError] = useState<ClientFailure | null>(null)
  const [imageStatuses, setImageStatuses] = useState<RemoteImageStatus[]>([])
  const [frameReady, setFrameReady] = useState(false)
  const [retrying, setRetrying] = useState(false)
  const [cooldown, setCooldown] = useState(false)
  const currentToken = useRef<string | null>(null)
  const loaded = useRef(false)
  const active = useRef(false)
  useEffect(() => {
    let live = true
    const release = (value: string) => { void client.releaseBody(emailId, value).catch(() => undefined) }
    active.current = true
    const seen = new Set<string>()
    const stop = client.onLinkRequest((event) => {
      if (!live || !active.current || !loaded.current || document.visibilityState === 'hidden' || event.emailId !== emailId ||
        event.documentToken !== currentToken.current || !event.requestId || seen.has(event.requestId) ||
        !Number.isSafeInteger(event.index) || event.index < 0 || !/^(https?:|mailto:)/i.test(event.url)) return
      seen.add(event.requestId)
      const { emailId: id, index, documentToken, requestId } = event
      void client.openLink({ emailId: id, index, documentToken, requestId }).catch((failure: unknown) => {
        if (live && active.current) setError(clientFailure(failure))
      })
    })
    const preparation = draftRevision !== undefined
      ? client.prepareDraftBody(emailId, draftRevision)
      : translation === undefined ? client.prepareBody(emailId, allowImages) : client.prepareBody(emailId, allowImages, translation)
    void preparation.then((prepared) => {
      if (!live) { release(prepared.documentToken); return }
      currentToken.current = prepared.documentToken
      setDocumentToken(prepared.documentToken)
    }).catch((failure: unknown) => {
      if (!live) return
      setError(clientFailure(failure))
      onPrepareFailed?.(clientFailure(failure))
    })
    return () => {
      live = false
      active.current = false
      stop()
      if (currentToken.current) release(currentToken.current)
      currentToken.current = null
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps -- the failure callback is only read when preparing fails
  }, [client, emailId, allowImages, draftRevision, translation])

  useEffect(() => {
    if (!cooldown) return
    const timer = setTimeout(() => setCooldown(false), 2000)
    return () => clearTimeout(timer)
  }, [cooldown])

  const retryImages = async () => {
    const previous = currentToken.current
    if (!previous || retrying || cooldown) return
    setRetrying(true)
    setCooldown(true)
    try {
      const prepared = await client.retryImages(emailId, previous)
      if (!active.current || currentToken.current !== previous) {
        void client.releaseBody(emailId, prepared.documentToken).catch(() => undefined)
        return
      }
      currentToken.current = prepared.documentToken
      loaded.current = false
      setFrameReady(false)
      setImageStatuses([])
      setDocumentToken(prepared.documentToken)
    } catch (failure) {
      if (active.current && currentToken.current === previous) setError(clientFailure(failure))
    } finally {
      if (active.current) setRetrying(false)
    }
  }

  useEffect(() => {
    if (!documentToken || !allowImages || !hasRemoteImages) return
    let live = true
    let timer: ReturnType<typeof setTimeout> | undefined
    const deadline = Date.now() + 35_000
    const poll = async () => {
      try {
        const statuses = await client.imageStatus(emailId, documentToken)
        if (!live) return
        setImageStatuses(statuses)
        if ((!frameReady || statuses.some((status) => status.loading)) && Date.now() < deadline) {
          timer = setTimeout(() => { void poll() }, 1000)
        }
      } catch {
        // A released/stale document has no readable diagnostics. Never display
        // raw IPC errors, or continue querying a binding after it was revoked.
      }
    }
    void poll()
    return () => { live = false; if (timer) clearTimeout(timer) }
  }, [client, emailId, documentToken, allowImages, hasRemoteImages, frameReady])

  const failedImages = imageStatuses.filter((status) => status.error !== null)
  const reasons = [...new Set(failedImages.map((status) => status.error).filter((reason): reason is ImageFetchError => reason !== null))]

  return <div className="flex min-h-0 flex-1 flex-col gap-2">
    {error && <Alert variant="destructive"><AlertTitle>{t('mail:bodyOrLinkFailed')}</AlertTitle><AlertDescription>{errorText(error)}</AlertDescription>
      {!documentToken && <Button size="sm" variant="outline" onClick={onReload}>{t('mail:retry')}</Button>}
    </Alert>}
    {failedImages.length > 0 && <div role="status" aria-label={t('mail:imageLoadingStatus')} className="flex shrink-0 items-center gap-3 rounded-md bg-muted px-3 py-2 text-xs">
      <span className="min-w-0 flex-1">{t('mail:imageFailureCount', { count: failedImages.length })} · {reasons.map((reason) => IMAGE_FAILURES[reason]).join('；')}</span>
      <Button size="sm" variant="outline" className="h-6 shrink-0 text-xs" disabled={retrying || cooldown || imageStatuses.some((status) => status.loading)} onClick={() => { void retryImages() }}>{t('mail:retryImages')}</Button>
    </div>}
    {!documentToken ? !error && <BodySkeleton /> :
      <iframe key={documentToken} title={title ?? t('mail:htmlBody')} sandbox="" referrerPolicy="no-referrer"
        src={bodyDocumentUrl(emailId, documentToken, dark)} className="min-h-0 w-full flex-1 border-0 bg-white"
        onLoad={() => {
          if (loaded.current) { active.current = false; onReload() }
          else { loaded.current = true; setFrameReady(true) }
        }} />}
  </div>
}

const IMAGE_FAILURES: Record<ImageFetchError, string> = {
  get targetBlocked() { return t('mail:imageAddressRestricted') },
  get proxyPolicyRequired() { return t('mail:allowImageDomainResolution') },
  get dns() { return t('mail:imageDnsFailed') },
  get connect() { return t('mail:networkConnectionFailed') },
  get proxy() { return t('mail:proxyConnectionFailed') },
  get tls() { return t('mail:imageCertificateFailed') },
  get httpStatus() { return t('mail:imageServerNoImage') },
  get notFound() { return t('mail:imageServerNoImage') },
  get unsupportedType() { return t('mail:unsupportedImageFormat') },
  get invalidImage() { return t('mail:imageDecodeFailed') },
  get tooLarge() { return t('mail:imageTooLarge') },
  get timeout() { return t('mail:imageTimedOut') },
  get cancelled() { return t('mail:imageExpired') },
}

export function BodySkeleton() {
  useTranslation()
  return <Delayed><div role="status" aria-label={t('mail:loadingBody')} className="flex flex-col gap-3"><Skeleton className="h-4 w-2/3" /><Skeleton className="h-40 w-full" /></div></Delayed>
}
