import { lazy, Suspense, useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { convertFileSrc, isTauri } from '@tauri-apps/api/core'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Delayed } from '@/components/ui/delayed'
import { Skeleton } from '@/components/ui/skeleton'
import type { ImageFetchError, RemoteImagePolicy, RemoteImageStatus } from '@/generated/accounts'
import { clientFailure, type MailxClient } from '@/lib/mailx-client'
import { useSettings, useUpdateSecurity } from '@/features/settings/settings-queries'

const DevelopmentBody = import.meta.env.DEV && !isTauri()
  ? lazy(() => import('./development-body')) : null

function bodyDocumentUrl(emailId: string, documentToken: string, dark: boolean) {
  const url = new URL(convertFileSrc('', 'mailx-body'))
  url.pathname = `/document/${encodeURIComponent(emailId)}`
  url.searchParams.set('token', documentToken)
  if (dark) url.searchParams.set('scheme', 'dark')
  return url.href
}

function subscribeVisibility(listener: () => void) {
  document.addEventListener('visibilitychange', listener)
  return () => document.removeEventListener('visibilitychange', listener)
}

function subscribeAppTheme(listener: () => void) {
  const observer = new MutationObserver(listener)
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] })
  return () => observer.disconnect()
}

export function BodyDocument({ client, emailId, hasRemoteImages = false }: { client: MailxClient; emailId: string; hasRemoteImages?: boolean }) {
  const visible = useSyncExternalStore(subscribeVisibility, () => document.visibilityState !== 'hidden')
  const settings = useSettings(client)
  const appDark = useSyncExternalStore(subscribeAppTheme, () => document.documentElement.classList.contains('dark'))
  const [version, setVersion] = useState(0)
  if (!visible) return null
  if (settings.isPending) return <BodySkeleton />
  // A settings read failure never silently grants remote access.
  const policy = settings.data?.settings.security.remoteImages ?? 'ask'
  const dark = appDark && (settings.data?.settings.appearance.mailTheme ?? 'follow') === 'follow'
  return <Reader key={`${emailId}:${version}:${policy}:${dark}`} client={client} emailId={emailId} policy={policy} dark={dark}
    hasRemoteImages={hasRemoteImages} onReload={() => setVersion((value) => value + 1)} />
}

function Reader({ client, emailId, policy, dark, hasRemoteImages, onReload }: {
  client: MailxClient; emailId: string; policy: RemoteImagePolicy; dark: boolean; hasRemoteImages: boolean; onReload: () => void
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
      <DocumentSession key={String(allowImages)} client={client} emailId={emailId} allowImages={allowImages} hasRemoteImages={hasRemoteImages} dark={dark} onReload={onReload} />}
  </div>
}

function DocumentSession({ client, emailId, allowImages, hasRemoteImages, dark, onReload }: {
  client: MailxClient; emailId: string; allowImages: boolean; hasRemoteImages: boolean; dark: boolean; onReload: () => void
}) {
  const [documentToken, setDocumentToken] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
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
        if (live && active.current) setError(clientFailure(failure).message)
      })
    })
    void client.prepareBody(emailId, allowImages).then((prepared) => {
      if (!live) { release(prepared.documentToken); return }
      currentToken.current = prepared.documentToken
      setDocumentToken(prepared.documentToken)
    }).catch((failure: unknown) => { if (live) setError(clientFailure(failure).message) })
    return () => {
      live = false
      active.current = false
      stop()
      if (currentToken.current) release(currentToken.current)
      currentToken.current = null
    }
  }, [client, emailId, allowImages])

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
      if (active.current && currentToken.current === previous) setError(clientFailure(failure).message)
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
    {error && <Alert variant="destructive"><AlertTitle>正文显示或链接操作失败</AlertTitle><AlertDescription>{error}</AlertDescription></Alert>}
    {failedImages.length > 0 && <div role="status" aria-label="图片加载状态" className="flex shrink-0 items-center gap-3 rounded-md bg-muted px-3 py-2 text-xs">
      <span className="min-w-0 flex-1">{failedImages.length} 张图片未能加载 · {reasons.map((reason) => IMAGE_FAILURES[reason]).join('；')}</span>
      <Button size="sm" variant="outline" className="h-6 shrink-0 text-xs" disabled={retrying || cooldown || imageStatuses.some((status) => status.loading)} onClick={() => { void retryImages() }}>重试图片</Button>
    </div>}
    {!documentToken ? !error && <BodySkeleton /> :
      <iframe key={documentToken} title="邮件 HTML 正文" sandbox="" referrerPolicy="no-referrer"
        src={bodyDocumentUrl(emailId, documentToken, dark)} className="min-h-0 w-full flex-1 border-0 bg-white"
        onLoad={() => {
          if (loaded.current) { active.current = false; onReload() }
          else { loaded.current = true; setFrameReady(true) }
        }} />}
  </div>
}

const IMAGE_FAILURES: Record<ImageFetchError, string> = {
  targetBlocked: '图片地址受限',
  proxyPolicyRequired: '请在账号代理设置中允许解析图片域名',
  dns: '无法解析图片服务器',
  connect: '网络连接失败',
  proxy: '代理连接失败',
  tls: '图片服务器证书验证失败',
  httpStatus: '图片服务器未返回图片',
  unsupportedType: '暂不支持此图片格式',
  invalidImage: '图片内容损坏或无法解码',
  tooLarge: '图片超过大小或解码限制',
  timeout: '图片加载超时',
  cancelled: '图片加载已失效，请重新打开邮件',
}

export function BodySkeleton() {
  return <Delayed><div role="status" aria-label="正在加载正文" className="flex flex-col gap-3"><Skeleton className="h-4 w-2/3" /><Skeleton className="h-40 w-full" /></div></Delayed>
}
