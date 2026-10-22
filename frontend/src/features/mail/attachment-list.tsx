import { useState } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import { Alert02Icon, Attachment01Icon, Download04Icon, Tick02Icon } from '@hugeicons/core-free-icons'
import type { AttachmentSummary } from '@/generated/accounts'
import { Spinner } from '@/components/ui/spinner'
import { clientFailure, type MailxClient } from '@/lib/mailx-client'
import { cn } from '@/lib/utils'
import { formatSize } from './format'

type SaveState = { kind: 'idle' } | { kind: 'saving' } | { kind: 'saved' } | { kind: 'failed'; message: string }

/** How long the "saved" tick stays before the button is plain again. */
const SAVED_VISIBLE_MS = 2500

export function AttachmentList({ client, emailId, attachments }: {
  client: MailxClient
  emailId: string
  attachments: AttachmentSummary[]
}) {
  return (
    <section className="flex max-h-36 shrink-0 flex-col gap-2 overflow-auto p-4" aria-label="附件">
      <h3 className="text-xs font-medium text-muted-foreground">{attachments.length} 个附件</h3>
      <ul className="flex flex-wrap gap-2">
        {attachments.map((attachment) => (
          <AttachmentItem key={attachment.id} client={client} emailId={emailId} attachment={attachment} />
        ))}
      </ul>
    </section>
  )
}

function AttachmentItem({ client, emailId, attachment }: {
  client: MailxClient
  emailId: string
  attachment: AttachmentSummary
}) {
  const [state, setState] = useState<SaveState>({ kind: 'idle' })
  const busy = state.kind === 'saving'
  const save = () => {
    if (busy) return
    setState({ kind: 'saving' })
    client.saveAttachment(emailId, attachment.id).then(
      (outcome) => {
        // Closing the dialog is not an error and needs no message.
        if (outcome === 'cancelled') return setState({ kind: 'idle' })
        setState({ kind: 'saved' })
        setTimeout(() => setState((current) => (current.kind === 'saved' ? { kind: 'idle' } : current)), SAVED_VISIBLE_MS)
      },
      (error: unknown) => setState({ kind: 'failed', message: clientFailure(error).message }),
    )
  }
  const icon = state.kind === 'saved' ? Tick02Icon : state.kind === 'failed' ? Alert02Icon : Download04Icon
  return (
    <li className="flex flex-col gap-1">
      <div className="flex items-center gap-2 rounded-lg border px-3 py-2 text-xs">
        <HugeiconsIcon icon={Attachment01Icon} className="size-3.5 shrink-0 text-muted-foreground" />
        <span className="max-w-60 break-all">{attachment.filename}</span>
        <span className="text-muted-foreground">{formatSize(attachment.size)}</span>
        <button
          type="button"
          onClick={save}
          disabled={busy}
          aria-label={`保存附件 ${attachment.filename}`}
          title="保存到…"
          className={cn(
            'flex size-6 shrink-0 items-center justify-center rounded-md text-muted-foreground outline-hidden transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:ring-2 focus-visible:ring-ring/50 disabled:pointer-events-none',
            state.kind === 'failed' && 'text-destructive',
          )}
        >
          {busy ? <Spinner className="size-3.5" aria-label="正在保存" /> : <HugeiconsIcon icon={icon} className="size-3.5" />}
        </button>
      </div>
      {state.kind === 'saved' && <span role="status" className="px-1 text-xs text-muted-foreground">已保存</span>}
      {state.kind === 'failed' && <span role="alert" className="px-1 text-xs text-destructive">{state.message}</span>}
    </li>
  )
}
