import { useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { RefreshIcon } from '@hugeicons/core-free-icons'
import { t } from '@/i18n'
import { IconButton } from '@/components/icon-button'
import { AiConsentDialog } from '@/features/ai/ai-consent-dialog'
import { AiGeneratedMark, AiMark } from '@/features/ai/ai-mark'
import { AiWorkingLine } from '@/features/ai/ai-working'
import { usePrefersReducedMotion } from '@/hooks/use-reduced-motion'
import type { MailxClient } from '@/lib/mailx-client'
import { cn } from '@/lib/utils'
import type { NowSummary } from './use-now-summary'

/**
 * The line under the greeting: the preset sentence, or the AI's once it is there (now-page.md 5.1).
 * Both sit in the same grid cell and cross-fade, so the line keeps its height until the AI text
 * needs more. While the run works, a thin AI line glides under it; the mark, the time tooltip and
 * the refresh button appear with the finished text.
 */
export function SummaryLine({ client, preset, summary }: { client: MailxClient; preset: ReactNode; summary: NowSummary }) {
  useTranslation()
  const reduced = usePrefersReducedMotion()
  const [asking, setAsking] = useState(false)
  const { text, working, completedAt, needsConsent, service, refresh } = summary
  const ai = text !== ''
  const done = completedAt !== null
  const fade = reduced ? 'transition-opacity duration-100' : 'transition-opacity duration-200 ease-[cubic-bezier(0.2,0,0,1)]'
  return (
    <div data-slot="now-summary" className="group/summary relative mt-2 flex flex-wrap items-baseline gap-x-3">
      <div className="relative grid min-w-0">
        <p data-slot="now-summary-preset" aria-hidden={ai || undefined} className={cn('col-start-1 row-start-1 min-h-6 text-base text-muted-foreground', fade, ai && 'opacity-0')}>{preset}</p>
        <div data-slot="now-summary-ai" data-shown={ai} className={cn('col-start-1 row-start-1 flex items-start gap-1.5 text-base text-muted-foreground', fade, !ai && 'pointer-events-none opacity-0')}>
          {ai && (done
            ? <span className="mt-[5px] inline-flex"><AiGeneratedMark at={completedAt} /></span>
            : <AiMark className="mt-[5px]" />)}
          <span className="min-w-0">
            <span aria-hidden>{text}</span>
            {done && (
              <IconButton label={t('now:aiRefresh')} size="icon-xs" onClick={refresh}
                className={cn('ml-1 align-middle opacity-0 group-focus-within/summary:opacity-100 group-hover/summary:opacity-100 focus-visible:opacity-100', fade)}>
                <HugeiconsIcon icon={RefreshIcon} strokeWidth={2} />
              </IconButton>
            )}
          </span>
          {/* Read out once, when it is complete, not word by word. */}
          <span role="status" className="sr-only">{done ? text : ''}</span>
        </div>
        <AiWorkingLine active={working} className="-bottom-1.5" />
      </div>
      {needsConsent && (
        <button type="button" onClick={() => setAsking(true)}
          className="inline-flex items-center gap-1.5 rounded-md px-1.5 py-0.5 text-sm text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring">
          <AiMark />{t('now:aiSummarize')}
        </button>
      )}
      <AiConsentDialog client={client} service={service} open={asking} onOpenChange={setAsking} onGranted={refresh} />
    </div>
  )
}
