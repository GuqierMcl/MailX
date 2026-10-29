import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { Alert02Icon, Attachment01Icon, Cancel01Icon } from '@hugeicons/core-free-icons'
import type { AttachmentPolicy, DraftAttachmentSummary } from '@/generated/accounts'
import { Button } from '@/components/ui/button'
import { Spinner } from '@/components/ui/spinner'
import { formatSize } from '@/features/mail/format'
import { cn } from '@/lib/utils'

type AttachmentBarProps = {
  attachments: DraftAttachmentSummary[]
  policy: AttachmentPolicy | null
  /** The draft is queued: attachments stay visible but cannot change. */
  readOnly: boolean
  /** An add or remove is in flight. */
  busy: boolean
  onRemove: (attachmentId: string) => void
}

/** What the attachments add up to against the core's limits; the core stays authoritative at send time. */
function attachmentStatus(attachments: DraftAttachmentSummary[], policy: AttachmentPolicy | null) {
  const total = attachments.reduce((sum, item) => sum + item.size, 0)
  const tooLarge = policy !== null && total > policy.maxTotalBytes
  const tooMany = policy !== null && attachments.length > policy.maxCount
  // The server limit is a hint (it only exists when the server announced one), never a block.
  const serverLimitMb = policy?.serverLimitMb ?? null
  const overServerHint = serverLimitMb !== null && total > serverLimitMb * 1024 * 1024
  return { total, tooLarge, tooMany, overServerHint, serverLimitMb }
}

/**
 * The files attached to the draft, between the editor and the bottom bar. Rows sit on the island's own surface:
 * no divider, no card border (ui-design.md).
 */
export function AttachmentBar({ attachments, policy, readOnly, busy, onRemove }: AttachmentBarProps) {
  const { t } = useTranslation('composer')
  if (attachments.length === 0 && !busy) return null
  const status = attachmentStatus(attachments, policy)
  const over = status.tooLarge || status.tooMany
  return (
    <div data-slot="composer-attachments" data-over-limit={over || undefined} className="flex shrink-0 flex-col gap-1 px-4 pb-1">
      <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
        <HugeiconsIcon icon={Attachment01Icon} className="size-3.5 shrink-0" />
        <span>{t('attachment.summary', { count: attachments.length, size: formatSize(status.total) })}</span>
        {busy && <Spinner className="size-3" aria-label={t('attachment.adding')} />}
      </div>
      {attachments.length > 0 && (
        <ul aria-label={t('attachment.list')} className="flex max-h-28 flex-col overflow-y-auto">
          {attachments.map((item) => (
            <li key={item.id} className="group flex items-center gap-2 rounded-lg px-2 py-0.5 text-sm hover:bg-muted/40">
              <span className="min-w-0 flex-1 truncate" title={item.fileName}>{item.fileName}</span>
              {item.origin === 'forwarded' && <span className="shrink-0 text-xs text-muted-foreground">{t('attachment.forwarded')}</span>}
              <span className="shrink-0 text-xs tabular-nums text-muted-foreground">{formatSize(item.size)}</span>
              <Button
                type="button"
                variant="ghost"
                size="icon-xs"
                disabled={readOnly || busy}
                aria-label={t('attachment.remove', { name: item.fileName })}
                onClick={() => onRemove(item.id)}
                className={cn('shrink-0', !readOnly && 'opacity-60 group-hover:opacity-100 focus-visible:opacity-100')}
              >
                <HugeiconsIcon icon={Cancel01Icon} className="size-3.5" />
              </Button>
            </li>
          ))}
        </ul>
      )}
      {over && policy && (
        <p role="alert" className="flex items-center gap-1 text-xs text-destructive">
          <HugeiconsIcon icon={Alert02Icon} className="size-3.5 shrink-0" />
          {status.tooLarge
            ? t('attachment.overSize', { limit: formatSize(policy.maxTotalBytes) })
            : t('attachment.overCount', { count: policy.maxCount })}
        </p>
      )}
      {!over && status.overServerHint && status.serverLimitMb !== null && (
        <p role="note" className="text-xs text-muted-foreground">{t('attachment.serverHint', { limit: status.serverLimitMb })}</p>
      )}
    </div>
  )
}
