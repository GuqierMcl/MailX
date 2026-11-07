import type { ComponentProps } from 'react'
import { useTranslation } from 'react-i18next'
import { IconButton } from '@/components/icon-button'
import { cn } from '@/lib/utils'

type ReadStatusButtonProps = Pick<ComponentProps<typeof IconButton>, 'disabled' | 'onClick'> & {
  isRead: boolean
  /** The card keeps its dot when the hover action is unmounted. */
  interactive?: boolean
  className?: string
  /** Fades only the ring and its button, leaving the blue dot stable. */
  ringClassName?: string
  'data-row-action'?: boolean
}

/** Shared read-state control: a 14px ring and 8px dot inside a 28px target. */
export function ReadStatusButton({ isRead, interactive = true, className, ringClassName, ...buttonProps }: ReadStatusButtonProps) {
  const { t } = useTranslation('mail')
  return (
    <span className={cn('relative inline-flex size-7 shrink-0 items-center justify-center', className)}>
      {!isRead && <span role="img" aria-label={t('unread')} className="pointer-events-none size-2 rounded-full bg-blue-500 dark:bg-blue-400" />}
      {interactive && (
        <span className={cn('absolute inset-0', ringClassName)}>
          <IconButton
            {...buttonProps}
            label={isRead ? t('markUnread') : t('markRead')}
            className="rounded-full bg-transparent p-0 text-foreground/30 transition-colors hover:bg-transparent hover:text-foreground/50 dark:hover:bg-transparent active:not-aria-[haspopup]:translate-y-0"
          >
            <span aria-hidden className="size-3.5 rounded-full border border-current" />
          </IconButton>
        </span>
      )}
    </span>
  )
}
