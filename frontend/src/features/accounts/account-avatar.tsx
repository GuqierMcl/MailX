import { cn } from '@/lib/utils'
import { accountInitial } from './account-color'
import { providerOf } from './providers'

/**
 * A bundled provider mark when enabled, otherwise the account's initial.
 * Its identification color stays visible around the provider's own artwork.
 */
export function AccountAvatar({ displayName, color, providerId, useProviderIcon = true, className }: {
  displayName: string
  color: string | null
  providerId?: string | null
  useProviderIcon?: boolean
  className?: string
}) {
  const provider = providerOf(providerId ?? null)
  const logo = useProviderIcon ? provider.logo : undefined
  return (
    <span
      aria-hidden
      data-slot="account-avatar"
      style={color ? { backgroundColor: color } : undefined}
      className={cn(
        'inline-flex size-6 shrink-0 items-center justify-center rounded-full text-xs font-medium select-none',
        color ? 'text-white' : 'bg-muted text-muted-foreground ring-1 ring-border ring-inset',
        logo && 'p-0.5',
        className,
      )}
    >
      {logo ? <img src={logo} alt="" aria-hidden className="size-full object-contain" /> : accountInitial(displayName)}
    </span>
  )
}
