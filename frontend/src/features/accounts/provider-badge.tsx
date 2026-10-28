import { HugeiconsIcon } from '@hugeicons/react'
import { cn } from '@/lib/utils'
import { providerOf, type MailProvider } from './providers'

function ProviderMark({ provider, className }: { provider: MailProvider; className: string }) {
  return provider.logo
    ? <img src={provider.logo} alt="" aria-hidden className={cn('object-contain', className)} />
    : <HugeiconsIcon icon={provider.icon} aria-hidden className={className} />
}

/** The provider's mark on a neutral tile; sized by the caller. */
export function ProviderIcon({ provider, className }: { provider: MailProvider; className?: string }) {
  return (
    <span aria-hidden className={cn('flex size-10 shrink-0 items-center justify-center rounded-md bg-muted', className)}>
      <ProviderMark provider={provider} className={provider.logo ? 'size-4/5' : 'size-1/2'} />
    </span>
  )
}

/** A compact provider tag: mark and name. */
export function ProviderBadge({ providerId, className }: { providerId: string | null; className?: string }) {
  const provider = providerOf(providerId)
  return (
    <span data-slot="provider-badge" className={cn('inline-flex items-center gap-1.5 rounded-md border px-1.5 py-0.5 text-xs font-normal text-muted-foreground', className)}>
      <ProviderMark provider={provider} className="size-3.5" />
      {provider.name}
    </span>
  )
}
