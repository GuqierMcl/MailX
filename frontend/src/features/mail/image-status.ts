import { createContext } from 'react'
import type { ImageFetchError, RemoteImageStatus } from '@/generated/accounts'

export const ImageDetailsSlot = createContext<HTMLElement | null>(null)

/** Requests for a repeated URL share one outcome. A successful copy can still be read. */
export function summarizeImageFailures(statuses: RemoteImageStatus[]) {
  const groups = new Map<number, RemoteImageStatus[]>()
  for (const status of statuses) {
    const copies = groups.get(status.group)
    if (copies) copies.push(status)
    else groups.set(status.group, [status])
  }
  let content = 0
  let failedContent = 0
  let failed = 0
  let importantFailed = false
  const reasons = new Set<ImageFetchError>()
  for (const copies of groups.values()) {
    if (copies.some((copy) => copy.loading)) continue
    const decorative = copies.every((copy) => copy.role === 'decorative')
    if (!decorative) content++
    if (copies.some((copy) => copy.error === null)) continue
    failed++
    for (const copy of copies) if (copy.error !== null) reasons.add(copy.error)
    if (!decorative) failedContent++
    if (copies.some((copy) => copy.role === 'important')) importantFailed = true
  }
  return { failed, reasons, severe: importantFailed || (failedContent >= 3 && failedContent * 2 >= content) }
}
