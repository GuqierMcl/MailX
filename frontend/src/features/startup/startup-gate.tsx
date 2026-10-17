import { useCallback, useEffect, useRef, useState } from 'react'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardDescription, CardFooter, CardHeader, CardTitle } from '@/components/ui/card'
import { FieldDescription } from '@/components/ui/field'
import { Skeleton } from '@/components/ui/skeleton'
import type { StartupStatus } from '@/generated/startup'
import { clientFailure, type ClientFailure, type MailxClient } from '@/lib/mailx-client'
import { AccountsPage } from '@/features/accounts/accounts-page'

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
  if (status?.state === 'ready') return <AccountsPage client={client} />
  if (status?.state === 'failed') {
    return <StartupFailureScreen status={status} retrying={retrying} onRetry={() => { void retryStartup() }} />
  }
  return <StartupLoadingScreen />
}

function StartupShell({ children }: { children: React.ReactNode }) {
  return <Card className="flex min-h-svh w-full max-w-none flex-col items-center justify-center rounded-none border-0 bg-background p-6 shadow-none [&>*]:w-full [&>*]:max-w-md">{children}</Card>
}

function StartupLoadingScreen() {
  return (
    <StartupShell>
      <CardHeader>
        <CardTitle>正在准备 MailX</CardTitle>
        <CardDescription>正在初始化本地数据，完成后即可进入邮箱工作台。</CardDescription>
      </CardHeader>
      <CardContent className="space-y-3" role="status" aria-label="正在初始化本地数据">
        <Skeleton className="h-3 w-4/5" />
        <Skeleton className="h-3 w-3/5" />
        <FieldDescription>请稍候，不会联网检查邮箱。</FieldDescription>
      </CardContent>
    </StartupShell>
  )
}

function UnavailableScreen() {
  return (
    <StartupShell>
      <CardHeader>
        <CardTitle>桌面能力不可用</CardTitle>
        <CardDescription>账号和本地数据只能在 MailX 桌面应用中使用。</CardDescription>
      </CardHeader>
      <CardContent>
        <Alert>
          <AlertTitle>请打开桌面应用</AlertTitle>
          <AlertDescription>当前页面没有连接到 MailX 宿主，未读取浏览器存储，也不会模拟本地账号。</AlertDescription>
        </Alert>
      </CardContent>
    </StartupShell>
  )
}

function TransportFailureScreen({ failure, onRetry }: { failure: QueryFailure, onRetry: () => void }) {
  return (
    <StartupShell>
      <CardHeader>
        <CardTitle>无法读取启动状态</CardTitle>
        <CardDescription>MailX 没有返回当前初始化状态。</CardDescription>
      </CardHeader>
      <CardContent>
        <Alert variant="destructive">
          <AlertTitle>{failure.message}</AlertTitle>
          <AlertDescription>这不代表迁移失败。请重新查询状态后再继续。</AlertDescription>
        </Alert>
      </CardContent>
      <CardFooter>
        <Button type="button" onClick={onRetry}>重新查询</Button>
      </CardFooter>
    </StartupShell>
  )
}

function StartupFailureScreen({ status, retrying, onRetry }: { status: Extract<StartupStatus, { state: 'failed' }>, retrying: boolean, onRetry: () => void }) {
  return (
    <StartupShell>
      <CardHeader>
        <CardTitle>MailX 尚未准备好</CardTitle>
        <CardDescription>本地数据初始化未完成，账号页面暂时不会打开。</CardDescription>
      </CardHeader>
      <CardContent>
        <Alert variant="destructive">
          <AlertTitle>{status.error.message}</AlertTitle>
          <AlertDescription>请处理提示的问题后重试。应用不会跳过数据库迁移。</AlertDescription>
        </Alert>
      </CardContent>
      {status.canRetry && (
        <CardFooter>
          <Button type="button" onClick={onRetry} disabled={retrying}>
            {retrying ? '正在重试…' : '重试初始化'}
          </Button>
        </CardFooter>
      )}
    </StartupShell>
  )
}
