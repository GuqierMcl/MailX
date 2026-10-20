import { lazy, Suspense, useEffect, useRef, useState, useSyncExternalStore } from 'react'
import { convertFileSrc, isTauri } from '@tauri-apps/api/core'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription,
  AlertDialogFooter, AlertDialogHeader, AlertDialogTitle } from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { Skeleton } from '@/components/ui/skeleton'
import { clientFailure, type BodyLinkRequest, type BodyLinks, type MailxClient } from '@/lib/mailx-client'

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

export function BodyDocument({ client, emailId }: { client: MailxClient; emailId: string }) {
  const visible = useSyncExternalStore(subscribeVisibility, () => document.visibilityState !== 'hidden')
  const [version, setVersion] = useState(0)
  if (!visible) return null
  if (DevelopmentBody) return <Suspense fallback={<BodySkeleton />}><DevelopmentBody /></Suspense>
  return <DocumentSession key={`${emailId}:${version}`} client={client} emailId={emailId} onReload={() => setVersion((value) => value + 1)} />
}

function DocumentSession({ client, emailId, onReload }: { client: MailxClient; emailId: string; onReload: () => void }) {
  const [documentData, setDocumentData] = useState<BodyLinks | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [request, setRequest] = useState<BodyLinkRequest | null>(null)
  const loaded = useRef(false)
  const active = useRef(false)
  const consumed = useRef(new Set<string>())
  useEffect(() => {
    let live = true
    let token: string | null = null
    active.current = true
    const seen = new Set<string>()
    const stop = client.onLinkRequest((event) => {
      if (!live || !active.current || !loaded.current || document.visibilityState === 'hidden' || event.emailId !== emailId ||
        event.documentToken !== token || !event.requestId || seen.has(event.requestId) ||
        !Number.isSafeInteger(event.index) || event.index < 0 || !/^(https?:|mailto:)/i.test(event.url)) return
      seen.add(event.requestId)
      setRequest(event)
    })
    void client.prepareBody(emailId).then(async (prepared) => {
      if (!live) return
      const links = await client.bodyLinks(emailId, prepared.documentToken)
      if (!live || links.documentToken !== prepared.documentToken) return
      token = prepared.documentToken
      setDocumentData(links)
    }).catch((failure: unknown) => { if (live) setError(clientFailure(failure).message) })
    return () => { live = false; active.current = false; stop() }
  }, [client, emailId])

  const confirm = () => {
    if (!request || !active.current || document.visibilityState === 'hidden' || consumed.current.has(request.requestId)) return
    consumed.current.add(request.requestId)
    setRequest(null)
    const { emailId: id, index, documentToken, requestId } = request
    void client.openLink({ emailId: id, index, documentToken, requestId }).catch((failure: unknown) => {
      if (active.current) setError(clientFailure(failure).message)
    })
  }
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2">
      <div className="flex justify-end"><Button size="sm" variant="ghost" onClick={onReload}>重新加载正文</Button></div>
      {error && <Alert variant="destructive"><AlertTitle>正文显示或链接操作失败</AlertTitle><AlertDescription>{error}</AlertDescription></Alert>}
      {!documentData ? !error && <BodySkeleton /> : <>
        <iframe title="邮件 HTML 正文" sandbox="" referrerPolicy="no-referrer"
          src={bodyDocumentUrl(emailId, documentData.documentToken)} className="min-h-0 w-full flex-1 border-0 bg-white"
          onLoad={() => {
            // A second navigation must rotate the binding before any new event
            // can authorize a link from the previous document.
            if (loaded.current) { active.current = false; onReload() }
            else loaded.current = true
          }} />
        {documentData.links.length > 0 && <section aria-label="正文链接" className="max-h-32 shrink-0 overflow-auto">
          <p className="text-xs text-muted-foreground">正文中的编号对应以下链接：</p>
          <ol className="flex flex-col items-start">
            {documentData.links.map((link) => <li key={link.index} className="max-w-full">
              <Button variant="link" size="sm" className="h-auto max-w-full whitespace-normal break-all text-left" onClick={() => {
                void client.requestLink({ emailId, index: link.index, documentToken: documentData.documentToken })
                  .catch((failure: unknown) => { if (active.current) setError(clientFailure(failure).message) })
              }}>[{link.index + 1}] {link.url}</Button>
            </li>)}
          </ol>
        </section>}
      </>}
      <AlertDialog open={request !== null} onOpenChange={(open) => { if (!open) setRequest(null) }}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>打开外部链接？</AlertDialogTitle>
            <AlertDialogDescription>即将使用系统应用打开以下完整地址，请确认目的地。</AlertDialogDescription>
          </AlertDialogHeader>
          <div className="max-h-60 overflow-auto whitespace-pre-wrap break-all text-sm"><bdi dir="ltr">{request?.url}</bdi></div>
          <AlertDialogFooter><AlertDialogCancel>取消</AlertDialogCancel><AlertDialogAction onClick={confirm}>确认打开</AlertDialogAction></AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

export function BodySkeleton() {
  return <div role="status" aria-label="正在加载正文" className="flex flex-col gap-3"><Skeleton className="h-4 w-2/3" /><Skeleton className="h-40 w-full" /></div>
}
