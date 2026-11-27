import { errorText } from '@/i18n/errors'
import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { useCallback, useEffect, useRef, useState } from 'react'
import { ErrorNotice } from '@/components/error-notice'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from '@/components/ui/card'
import type { StartupStatus } from '@/generated/startup'
import { clientFailure, type ClientFailure, type MailxClient } from '@/lib/mailx-client'
import { Workbench } from '@/features/mail/workbench'

const POLL_INTERVAL_MS = 500

type QueryFailure = ClientFailure

export function StartupGate({ client, children }: { client: MailxClient; children?: React.ReactNode }) {
  const [status, setStatus] = useState<StartupStatus | null>(null)
  const [queryFailure, setQueryFailure] = useState<QueryFailure | null>(null)
  const [retrying, setRetrying] = useState(false)
  const revisionRef = useRef(-1)
  const generationRef = useRef(0)
  const requestSerialRef = useRef(0)
  const inFlightRef = useRef(false)
  const mountedRef = useRef(false)

  const acceptStatus = useCallback((next: StartupStatus) => {
    if (next.revision < revisionRef.current) return false
    revisionRef.current = next.revision
    setStatus(next)
    setQueryFailure(null)
    return true
  }, [])

  const queryStatus = useCallback(async () => {
    if (!client.available || !mountedRef.current || inFlightRef.current) return
    const generation = generationRef.current
    const serial = ++requestSerialRef.current
    inFlightRef.current = true
    try {
      const next = await client.getStartupStatus()
      if (generationRef.current !== generation || !mountedRef.current) return
      acceptStatus(next)
    } catch (cause) {
      if (generationRef.current === generation && mountedRef.current) {
        setQueryFailure(clientFailure(cause))
      }
    } finally {
      if (generationRef.current === generation && requestSerialRef.current === serial) {
        inFlightRef.current = false
      }
    }
  }, [acceptStatus, client])

  useEffect(() => {
    if (!client.available) return
    mountedRef.current = true
    generationRef.current += 1
    void queryStatus()
    return () => {
      mountedRef.current = false
      generationRef.current += 1
      requestSerialRef.current += 1
      inFlightRef.current = false
    }
  }, [client, queryStatus])

  useEffect(() => {
    if (!client.available || queryFailure || status?.state !== 'initializing') return
    const timer = window.setTimeout(() => { void queryStatus() }, POLL_INTERVAL_MS)
    return () => window.clearTimeout(timer)
  }, [client.available, queryFailure, queryStatus, status])

  const retryStartup = useCallback(async () => {
    if (retrying || status?.state !== 'failed' || !status.canRetry) return
    setRetrying(true)
    setQueryFailure(null)
    try {
      const next = await client.retryStartup()
      acceptStatus(next)
    } catch {
      // A lost retry response does not prove that a retry failed. Read the
      // authoritative snapshot before offering another retry.
      await queryStatus()
    } finally {
      setRetrying(false)
    }
  }, [acceptStatus, client, queryStatus, retrying, status])

  if (!client.available) return <UnavailableScreen />
  if (queryFailure) return <TransportFailureScreen failure={queryFailure} onRetry={() => { void queryStatus() }} />
  if (status?.state === 'ready') return <><Workbench client={client} />{children}</>
  if (status?.state === 'failed') {
    return <StartupFailureScreen status={status} retrying={retrying} onRetry={() => { void retryStartup() }} />
  }
  return <StartupLoadingScreen />
}

function StartupShell({ children }: { children: React.ReactNode }) {
  return (
    <div className="h-full overflow-auto">
      <Card className="flex min-h-full w-full max-w-none flex-col items-center justify-center rounded-none border-0 bg-transparent p-6 pb-16 shadow-none [&>*]:w-full [&>*]:max-w-md">{children}</Card>
    </div>
  )
}

function StartupLoadingScreen() {
  useTranslation()
  return (
    <div
      className="flex h-full w-full select-none items-center justify-center pb-10"
      role="status"
      aria-label={t('common:initializingData')}
    >
      <div className="relative flex items-center justify-center">
        <span
          aria-hidden="true"
          className="startup-logo-glow pointer-events-none absolute -inset-6 rounded-full bg-foreground/[0.07] blur-2xl dark:bg-white/[0.12]"
        />
        <img
          src={`${import.meta.env.BASE_URL}logo.svg`}
          alt=""
          width={72}
          height={72}
          draggable={false}
          className="startup-logo-breathe relative size-[72px] drop-shadow-md"
        />
      </div>
    </div>
  )
}

function UnavailableScreen() {
  useTranslation()
  return (
    <StartupShell>
      <CardHeader>
        <CardTitle>{t('common:desktopUnavailable')}</CardTitle>
        <CardDescription>{t('common:desktopOnlyHint')}</CardDescription>
      </CardHeader>
      <CardContent>
        <Alert>
          <AlertTitle>{t('common:openDesktopApp')}</AlertTitle>
          <AlertDescription>{t('common:hostDisconnectedHint')}</AlertDescription>
        </Alert>
      </CardContent>
    </StartupShell>
  )
}

function TransportFailureScreen({ failure, onRetry }: { failure: QueryFailure, onRetry: () => void }) {
  useTranslation()
  return (
    <StartupShell>
      <CardHeader className="text-center">
        <CardTitle>{t('common:startupStatusFailed')}</CardTitle>
        <CardDescription>{t('common:startupStatusMissing')}</CardDescription>
      </CardHeader>
      <CardContent>
        <ErrorNotice layout="centered" message={errorText(failure)} detail={t('common:startupStatusRetryHint')}
          actionLabel={t('common:queryAgain')} onAction={onRetry} />
      </CardContent>
    </StartupShell>
  )
}

function StartupFailureScreen({ status, retrying, onRetry }: { status: Extract<StartupStatus, { state: 'failed' }>, retrying: boolean, onRetry: () => void }) {
  useTranslation()
  return (
    <StartupShell>
      <CardHeader className="text-center">
        <CardTitle>{t('common:notReady')}</CardTitle>
        <CardDescription>{t('common:notReadyDescription')}</CardDescription>
      </CardHeader>
      <CardContent>
        <ErrorNotice layout="centered" message={errorText(status.error)} detail={t('common:initializationRetryHint')}
          actionLabel={retrying ? t('common:retrying') : t('common:retryInitialization')}
          onAction={status.canRetry ? onRetry : undefined} actionDisabled={retrying} />
      </CardContent>
    </StartupShell>
  )
}
