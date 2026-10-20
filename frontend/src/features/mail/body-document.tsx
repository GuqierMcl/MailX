import { lazy, Suspense, useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { convertFileSrc, isTauri } from '@tauri-apps/api/core'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Skeleton } from '@/components/ui/skeleton'
import type { RemoteImagePolicy } from '@/generated/accounts'
import { clientFailure, type MailxClient } from '@/lib/mailx-client'
import { useSettings, useUpdateSecurity } from '@/features/settings/settings-queries'

const DevelopmentBody = import.meta.env.DEV && !isTauri()
  ? lazy(() => import('./development-body')) : null

function bodyDocumentUrl(emailId: string, documentToken: string) {
  const url = new URL(convertFileSrc('', 'mailx-body'))
  url.pathname = `/document/${encodeURIComponent(emailId)}`
  url.searchParams.set('token', documentToken)
  return url.href
}

function subscribeVisibility(listener: () => void) {
  document.addEventListener('visibilitychange', listener)
  return () => document.removeEventListener('visibilitychange', listener)
}

export function BodyDocument({ client, emailId, hasRemoteImages = false }: { client: MailxClient; emailId: string; hasRemoteImages?: boolean }) {
  const visible = useSyncExternalStore(subscribeVisibility, () => document.visibilityState !== 'hidden')
  const settings = useSettings(client)
  const [version, setVersion] = useState(0)
  if (!visible) return null
  if (settings.isPending) return <BodySkeleton />
  // A settings read failure never silently grants remote access.
  const policy = settings.data?.settings.security.remoteImages ?? 'ask'
  return <Reader key={`${emailId}:${version}:${policy}`} client={client} emailId={emailId} policy={policy}
    hasRemoteImages={hasRemoteImages} onReload={() => setVersion((value) => value + 1)} />
}

function Reader({ client, emailId, policy, hasRemoteImages, onReload }: {
  client: MailxClient; emailId: string; policy: RemoteImagePolicy; hasRemoteImages: boolean; onReload: () => void
}) {
  const [granted, setGranted] = useState(false)
  const [dismissed, setDismissed] = useState(false)
  const [remember, setRemember] = useState(false)
  const update = useUpdateSecurity(client)
  const allowImages = policy === 'automatic' || (policy === 'ask' && granted)
  const load = () => {
    if (remember) update.mutate({ remoteImages: 'automatic' })
    else setGranted(true)
  }
  return <div className="flex min-h-0 flex-1 flex-col gap-2">
    {policy === 'ask' && hasRemoteImages && !granted && !dismissed &&
      <div role="status" aria-label="远程图片提示" className="flex h-9 w-full shrink-0 items-center gap-3 rounded-md bg-muted px-3 text-xs">
        <span className="min-w-0 flex-1 truncate" title="此邮件包含远程图片。">此邮件包含远程图片。</span>
        <label className="flex shrink-0 cursor-pointer items-center gap-1.5"><Checkbox checked={remember} onCheckedChange={(value) => setRemember(value === true)} disabled={update.isPending} />不再提示</label>
        <Button size="sm" variant="outline" className="h-6 shrink-0 text-xs" onClick={load} disabled={update.isPending}>{update.isPending ? '保存中…' : '加载图片'}</Button>
        <Button size="icon-sm" variant="ghost" className="size-6 shrink-0" aria-label="关闭远程图片提示" onClick={() => setDismissed(true)} disabled={update.isPending}>×</Button>
      </div>}
    {update.isError && <Alert variant="destructive"><AlertTitle>设置保存失败</AlertTitle><AlertDescription>{clientFailure(update.error).message}</AlertDescription></Alert>}
    {DevelopmentBody ? <Suspense fallback={<BodySkeleton />}><DevelopmentBody /></Suspense> :
      <DocumentSession key={String(allowImages)} client={client} emailId={emailId} allowImages={allowImages} onReload={onReload} />}
  </div>
}

function DocumentSession({ client, emailId, allowImages, onReload }: { client: MailxClient; emailId: string; allowImages: boolean; onReload: () => void }) {
  const [documentToken, setDocumentToken] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const loaded = useRef(false)
  const active = useRef(false)
  useEffect(() => {
    let live = true
    let token: string | null = null
    const release = (value: string) => { void client.releaseBody(emailId, value).catch(() => undefined) }
    active.current = true
    const seen = new Set<string>()
    const stop = client.onLinkRequest((event) => {
      if (!live || !active.current || !loaded.current || document.visibilityState === 'hidden' || event.emailId !== emailId ||
        event.documentToken !== token || !event.requestId || seen.has(event.requestId) ||
        !Number.isSafeInteger(event.index) || event.index < 0 || !/^(https?:|mailto:)/i.test(event.url)) return
      seen.add(event.requestId)
      const { emailId: id, index, documentToken, requestId } = event
      void client.openLink({ emailId: id, index, documentToken, requestId }).catch((failure: unknown) => {
        if (live && active.current) setError(clientFailure(failure).message)
      })
    })
    void client.prepareBody(emailId, allowImages).then((prepared) => {
      if (!live) { release(prepared.documentToken); return }
      token = prepared.documentToken
      setDocumentToken(token)
    }).catch((failure: unknown) => { if (live) setError(clientFailure(failure).message) })
    return () => {
      live = false
      active.current = false
      stop()
      if (token) release(token)
    }
  }, [client, emailId, allowImages])

  return <div className="flex min-h-0 flex-1 flex-col gap-2">
    {error && <Alert variant="destructive"><AlertTitle>正文显示或链接操作失败</AlertTitle><AlertDescription>{error}</AlertDescription></Alert>}
    {!documentToken ? !error && <BodySkeleton /> :
      <iframe title="邮件 HTML 正文" sandbox="" referrerPolicy="no-referrer"
        src={bodyDocumentUrl(emailId, documentToken)} className="min-h-0 w-full flex-1 border-0 bg-white"
        onLoad={() => {
          if (loaded.current) { active.current = false; onReload() }
          else loaded.current = true
        }} />}
  </div>
}

export function BodySkeleton() {
  return <div role="status" aria-label="正在加载正文" className="flex flex-col gap-3"><Skeleton className="h-4 w-2/3" /><Skeleton className="h-40 w-full" /></div>
}
