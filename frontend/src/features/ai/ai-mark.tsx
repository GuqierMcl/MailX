import { useId } from 'react'
import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'

/**
 * The AI sparkle, filled with the AI gradient (index.css `--ai-*`). Decorative: it never carries
 * meaning alone, so it is hidden from screen readers; use `AiGeneratedMark` where it says "made by AI".
 */
export function AiMark({ className }: { className?: string }) {
  const id = useId()
  return (
    <svg aria-hidden data-slot="ai-mark" viewBox="0 0 24 24" className={cn('size-3.5 shrink-0', className)}>
      <defs>
        <linearGradient id={id} x1="2" y1="22" x2="22" y2="2" gradientUnits="userSpaceOnUse">
          <stop offset="0" style={{ stopColor: 'var(--ai-1)' }} />
          <stop offset="0.55" style={{ stopColor: 'var(--ai-2)' }} />
          <stop offset="1" style={{ stopColor: 'var(--ai-3)' }} />
        </linearGradient>
      </defs>
      <path fill={`url(#${id})`} d="M12 1.5Q13.2 10.8 22.5 12Q13.2 13.2 12 22.5Q10.8 13.2 1.5 12Q10.8 10.8 12 1.5Z" />
    </svg>
  )
}

/** The mark with "由 AI 生成 · 生成于 10:24" on hover and keyboard focus; the same words name it for screen readers. */
export function AiGeneratedMark({ at, className }: { at?: Date | null; className?: string }) {
  const { i18n } = useTranslation()
  const time = at ? new Intl.DateTimeFormat(i18n.language, { hour: '2-digit', minute: '2-digit' }).format(at) : null
  const label = time ? t('ai:generatedAt', { time }) : t('ai:generated')
  return (
    <Tooltip>
      <TooltipTrigger render={<span role="img" aria-label={label} tabIndex={0} className="inline-flex rounded-sm outline-none focus-visible:ring-2 focus-visible:ring-ring" />}>
        <AiMark className={className} />
      </TooltipTrigger>
      <TooltipContent>{label}</TooltipContent>
    </Tooltip>
  )
}
