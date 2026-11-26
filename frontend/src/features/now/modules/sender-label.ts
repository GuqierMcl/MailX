import type { OverviewSender } from '@/generated/accounts'

/** What a sender is called: the name they gave (untrusted, shown as plain text), else the address. */
export function senderLabel(sender: OverviewSender): string {
  return sender.name?.trim() || sender.address
}
