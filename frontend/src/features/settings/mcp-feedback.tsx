import type { ReactNode } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import { Alert02Icon, CheckmarkCircle02Icon, InformationCircleIcon } from '@hugeicons/core-free-icons'

/** Small operation feedback, using the same icons and colors as the app's toasts. */
export function McpFeedback({ tone = 'neutral', children }: {
  tone?: 'neutral' | 'success' | 'error'
  children: ReactNode
}) {
  const icon = tone === 'error' ? Alert02Icon : tone === 'success' ? CheckmarkCircle02Icon : InformationCircleIcon
  return <span role={tone === 'error' ? 'alert' : 'status'} className="flex min-w-0 items-start gap-1.5 text-xs/relaxed text-muted-foreground">
    <HugeiconsIcon icon={icon} strokeWidth={1.75} aria-hidden className={`mt-0.5 size-3.5 shrink-0 ${tone === 'error' ? 'text-destructive' : tone === 'success' ? 'text-emerald-600 dark:text-emerald-400' : ''}`} />
    <span className="min-w-0 break-words">{children}</span>
  </span>
}
