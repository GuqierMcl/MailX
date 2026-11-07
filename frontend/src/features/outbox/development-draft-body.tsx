// Browser-only approximation for synthetic drafts; never a desktop fallback.
import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { Draft } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'
import { mockDraftBody } from '@/lib/mock-draft-body'
import editorCss from '@/features/composer/mail-body-editor.css?raw'
import { BodySkeleton } from '@/features/mail/body-document'

export default function DevelopmentDraftBody({ client, draft, dark }: { client: MailxClient; draft: Draft; dark: boolean }) {
  const { t } = useTranslation('mail')
  const [html, setHtml] = useState<string | null>(null)
  useEffect(() => {
    let live = true
    const urls: string[] = []
    const template = document.createElement('template')
    template.innerHTML = mockDraftBody(draft.bodyHtml).bodyHtml
    // Navigation in the browser approximation cannot emulate the host's link handshake.
    for (const link of template.content.querySelectorAll('a')) link.removeAttribute('href')
    void Promise.all(Array.from(template.content.querySelectorAll('img')).map(async (image) => {
      const cid = image.getAttribute('src')?.slice(4)
      image.removeAttribute('src')
      const attachment = draft.attachments.find((file) => file.inline && file.contentId === cid)
      if (!attachment) return
      try {
        const bytes = await client.inlineImagePreview(draft.id, attachment.id)
        if (!live) return
        const url = URL.createObjectURL(new Blob([bytes], { type: attachment.mediaType }))
        urls.push(url)
        image.src = url
      } catch { /* The image's alt text remains visible. */ }
    })).then(() => {
      if (live) setHtml(template.innerHTML)
    })
    return () => { live = false; urls.forEach((url) => URL.revokeObjectURL(url)) }
  }, [client, draft])
  if (html === null) return <BodySkeleton />
  return <iframe title={t('htmlBody')} sandbox="" referrerPolicy="no-referrer" className="min-h-0 w-full flex-1 border-0"
    srcDoc={`<!doctype html><html class="${dark ? 'dark' : ''}"><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; img-src blob:"><style>${editorCss}
body{margin:0;background:${dark ? '#171717' : '#fff'};color:${dark ? '#e5e5e5' : '#222'};font-family:system-ui,sans-serif}.mail-body-editor{padding:0}img{max-width:100%;height:auto}a{color:#3b82f6;text-decoration:underline}</style></head><body><div class="mail-body-editor"><div class="mail-body-content">${html}</div></div></body></html>`} />
}
