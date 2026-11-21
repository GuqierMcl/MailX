import type { ReactNode } from 'react'
import { RefreshIcon } from '@hugeicons/core-free-icons'
import { HugeiconsIcon } from '@hugeicons/react'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'

type ErrorNoticeProps = {
  layout?: 'centered' | 'inline'
  message: ReactNode
  detail?: ReactNode
  actionLabel?: string
  onAction?: () => void
  actionDisabled?: boolean
  children?: ReactNode
}

export function ErrorNotice({ layout = 'inline', message, detail, actionLabel, onAction, actionDisabled, children }: ErrorNoticeProps) {
  const centered = layout === 'centered'
  return (
    <div className={cn('min-w-0 text-sm text-muted-foreground',
      centered && 'flex min-h-0 flex-1 flex-col overflow-y-auto p-2 text-center')}>
      <div className={cn('flex min-w-0 gap-y-2',
        centered ? 'm-auto max-w-sm shrink-0 flex-col items-center' : 'flex-wrap items-center gap-x-2')}>
        <div role="alert" className="min-w-0 max-w-full wrap-anywhere">
          <div>{message}</div>
          {detail && <div className="mt-1 text-xs/relaxed">{detail}</div>}
        </div>
        {children}
        {actionLabel && onAction && (
          <Button type="button" variant="link" className="h-auto min-h-7 max-w-full whitespace-normal" disabled={actionDisabled} onClick={onAction}>
            <HugeiconsIcon icon={RefreshIcon} aria-hidden="true" />
            {actionLabel}
          </Button>
        )}
      </div>
    </div>
  )
}
