import { HugeiconsIcon } from '@hugeicons/react'
import { Attachment01Icon } from '@hugeicons/core-free-icons'
import { useTranslation } from 'react-i18next'
import type { DraftAttachmentSummary } from '@/generated/accounts'
import { formatSize } from '@/features/mail/format'

export function DraftAttachmentList({ attachments }: { attachments: DraftAttachmentSummary[] }) {
  const { t } = useTranslation('mail')
  const files = attachments.filter((attachment) => !attachment.inline)
  if (!files.length) return null
  return <section className="flex max-h-36 shrink-0 flex-col gap-2 overflow-auto p-4" aria-label={t('attachments')}>
    <h3 className="text-xs font-medium text-muted-foreground">{t('attachmentCount', { count: files.length })}</h3>
    <ul className="flex flex-wrap gap-x-6 gap-y-2">
      {files.map((file) => <li key={file.id} className="flex min-w-0 items-center gap-2 text-xs">
        <HugeiconsIcon icon={Attachment01Icon} className="size-3.5 shrink-0 text-muted-foreground" />
        <span className="max-w-60 break-all">{file.fileName || t('unnamedAttachment')}</span>
        <span className="shrink-0 text-muted-foreground">{formatSize(file.size)}</span>
      </li>)}
    </ul>
  </section>
}
