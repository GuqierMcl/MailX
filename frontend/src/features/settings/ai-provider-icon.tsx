import { useQuery } from '@tanstack/react-query'
import type { MailxClient } from '@/lib/mailx-client'
import { aiKeys } from '@/lib/query-client'
import { cn } from '@/lib/utils'

/** Logos bundled for the built-in providers, by provider id (see `assets/ai-providers/README.md`). */
const BUNDLED = import.meta.glob<string>('../../assets/ai-providers/*.svg', { eager: true, query: '?url', import: 'default' })
const BUNDLED_BY_ID = new Map(Object.entries(BUNDLED).map(([path, url]) => [/([^/]+)\.svg$/.exec(path)?.[1] ?? '', url]))

const BASE64 = /^[A-Za-z0-9+/]+={0,2}$/

/**
 * The provider's logo, or the first letter of its name.
 *
 * Logos are monochrome SVGs used only as a CSS mask over the current text color,
 * so they follow the theme. The SVG is never put into the page as markup, and
 * the mask is a `data:` or bundled URL that the app's img-src allows.
 */
export function AiProviderIcon({ client, providerId, name, className }: {
  client: MailxClient
  providerId: string
  name: string
  className?: string
}) {
  const bundled = BUNDLED_BY_ID.get(providerId)
  const downloaded = useQuery({
    queryKey: aiKeys.icon(providerId),
    queryFn: () => client.aiProviderIcon(providerId),
    enabled: bundled === undefined && providerId !== 'custom',
    staleTime: 24 * 60 * 60 * 1000,
    retry: false,
  })
  const icon = downloaded.data
  const url = bundled ?? (icon && icon.mimeType === 'image/svg+xml' && BASE64.test(icon.data) ? `data:image/svg+xml;base64,${icon.data}` : null)
  if (url === null) {
    return <span aria-hidden data-testid="ai-provider-initial"
      className={cn('inline-flex size-6 shrink-0 items-center justify-center rounded-md bg-muted text-xs font-medium text-muted-foreground', className)}>
      {Array.from(name.trim())[0]?.toUpperCase() ?? '?'}
    </span>
  }
  const mask = `url("${url}")`
  return <span aria-hidden data-testid="ai-provider-mask" className={cn('inline-block size-6 shrink-0 bg-current', className)}
    style={{ maskImage: mask, WebkitMaskImage: mask, maskSize: 'contain', WebkitMaskSize: 'contain', maskRepeat: 'no-repeat', WebkitMaskRepeat: 'no-repeat', maskPosition: 'center', WebkitMaskPosition: 'center' }} />
}
