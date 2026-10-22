import { cn } from '@/lib/utils'
import { accountInitial } from './account-color'

/**
 * The account's mark: its initial on its identification color, or on a
 * neutral surface when it has none.
 */
export function AccountAvatar({ displayName, color, className }: {
  displayName: string
  color: string | null
  className?: string
}) {
  return (
    <span
      aria-hidden
      data-slot="account-avatar"
      style={color ? { backgroundColor: color } : undefined}
      className={cn(
        'inline-flex size-6 shrink-0 items-center justify-center rounded-full text-xs font-medium select-none',
        color ? 'text-white' : 'bg-muted text-muted-foreground ring-1 ring-border ring-inset',
        className,
      )}
    >
      {accountInitial(displayName)}
    </span>
  )
}
