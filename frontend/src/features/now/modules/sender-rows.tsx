import type { ReactNode } from 'react'
import type { OverviewSender } from '@/generated/accounts'
import { SenderAvatar } from '@/features/mail/sender-avatar'
import { senderLabel } from './sender-label'

/** The avatar and name of a sender row; the caller adds what follows. */
export function SenderName({ sender, accountId }: { sender: OverviewSender; accountId: string }) {
  return (
    <>
      <SenderAvatar sender={{ name: sender.name, address: sender.address }} accountId={accountId} size="sm" />
      <span className="min-w-0 flex-1 truncate font-medium">{senderLabel(sender)}</span>
    </>
  )
}

export function SenderList({ children }: { children: ReactNode }) {
  return <ul className="flex flex-col">{children}</ul>
}
