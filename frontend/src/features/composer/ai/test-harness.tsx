import { useRef, useState } from 'react'
import type { Draft } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'
import Editor, { type MailBodyEditorHandle } from '../mail-body-editor'

/** The editor with writing help, the way the composer mounts it, minus the saving around it. */
export function MailBodyEditor({ client, draft, handleRef }: { client: MailxClient; draft: Draft; handleRef?: { current: MailBodyEditorHandle | null } }) {
  const [html, setHtml] = useState(draft.bodyHtml)
  const local = useRef<MailBodyEditorHandle | null>(null)
  return <Editor onChange={setHtml} value={html} editorRef={handleRef ?? local}
    ai={{ client, draftId: draft.id, kind: draft.kind }} />
}
