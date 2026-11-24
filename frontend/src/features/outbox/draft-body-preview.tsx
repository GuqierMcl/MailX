import { lazy, Suspense, useState } from 'react'
import type { Draft } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'
import { mockBackend } from '@/lib/dev-mock'
import { BodySession, BodySkeleton } from '@/features/mail/body-document'
import { useBodyEnvironment } from '@/features/mail/use-body-environment'

const DevelopmentDraftBody = mockBackend ? lazy(() => import('./development-draft-body')) : null

export function DraftBodyPreview({ client, draft, onRefresh }: { client: MailxClient; draft: Draft; onRefresh: () => void }) {
  const { visible, settings, dark } = useBodyEnvironment(client)
  const [version, setVersion] = useState(0)
  if (!visible) return null
  if (settings.isPending) return <BodySkeleton />
  const reload = () => { onRefresh(); setVersion((value) => value + 1) }
  return DevelopmentDraftBody
    ? <Suspense fallback={<BodySkeleton />}><DevelopmentDraftBody key={`${draft.id}:${draft.revision}:${dark}`} client={client} draft={draft} dark={dark} /></Suspense>
    : <BodySession key={`${draft.id}:${draft.revision}:${dark}:${version}`} client={client} emailId={draft.id}
      draftRevision={draft.revision} dark={dark} onReload={reload} />
}
