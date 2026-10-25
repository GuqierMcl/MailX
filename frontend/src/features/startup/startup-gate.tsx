import { errorText } from '@/i18n/errors'
import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { useCallback, useEffect, useRef, useState } from 'react'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from '@/components/ui/card'
import { FieldDescription } from '@/components/ui/field'
import { Skeleton } from '@/components/ui/skeleton'
import type { StartupStatus } from '@/generated/startup'
import { clientFailure, type ClientFailure, type MailxClient } from '@/lib/mailx-client'
import { Workbench } from '@/features/mail/workbench'

const POLL_INTERVAL_MS = 500

type QueryFailure = ClientFailure

export function StartupGate({ client }: { client: MailxClient }) {
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
  if (status?.state === 'ready') return <Workbench client={client} />
  if (status?.state === 'failed') {
    return <StartupFailureScreen status={status} retrying={retrying} onRetry={() => { void retryStartup() }} />
  }
  return <StartupLoadingScreen />
}

function StartupShell({ children }: { children: React.ReactNode }) {
  return <Card className="flex min-h-full w-full max-w-none flex-col items-center justify-center rounded-none border-0 bg-background p-6 shadow-none [&>*]:w-full [&>*]:max-w-md">{children}</Card>
}

function StartupLoadingScreen() {
  useTranslation()
  return (
    <StartupShell>
      <CardHeader>
        <CardTitle>{t('common:preparingMailx')}</CardTitle>
        <CardDescription>{t('common:initializingDescription')}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-3" role="status" aria-label={t('common:initializingData')}>
        <Skeleton className="h-3 w-4/5" />
        <Skeleton className="h-3 w-3/5" />
        <FieldDescription>{t('common:initializingHint')}</FieldDescription>
      </CardContent>
    </StartupShell>
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
      <CardHeader>
        <CardTitle>{t('common:startupStatusFailed')}</CardTitle>
        <CardDescription>{t('common:startupStatusMissing')}</CardDescription>
      </CardHeader>
      <CardContent>
        <Alert variant="destructive">
          <AlertTitle>{errorText(failure)}</AlertTitle>
          <AlertDescription>{t('common:startupStatusRetryHint')}</AlertDescription>
        </Alert>
      </CardContent>
      <CardFooter>
        <Button type="button" onClick={onRetry}>{t('common:queryAgain')}</Button>
      </CardFooter>
    </StartupShell>
  )
}

function StartupFailureScreen({ status, retrying, onRetry }: { status: Extract<StartupStatus, { state: 'failed' }>, retrying: boolean, onRetry: () => void }) {
  useTranslation()
  return (
    <StartupShell>
      <CardHeader>
        <CardTitle>{t('common:notReady')}</CardTitle>
        <CardDescription>{t('common:notReadyDescription')}</CardDescription>
      </CardHeader>
      <CardContent>
        <Alert variant="destructive">
          <AlertTitle>{errorText(status.error)}</AlertTitle>
          <AlertDescription>{t('common:initializationRetryHint')}</AlertDescription>
        </Alert>
      </CardContent>
      {status.canRetry && (
        <CardFooter>
          <Button type="button" onClick={onRetry} disabled={retrying}>
            {retrying ? t('common:retrying') : t('common:retryInitialization')}
          </Button>
        </CardFooter>
      )}
    </StartupShell>
  )
}
