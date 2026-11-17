import { useEffect, useRef, useState, type ReactNode } from 'react'
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
import type { EntranceMode } from './motion'

const SHEEN_BACKGROUND = 'linear-gradient(100deg, transparent 38%, var(--ai-1) 44%, var(--ai-2) 50%, var(--ai-3) 56%, transparent 62%)'
const SWAP_MS = 200

/**
 * The line under the greeting: the preset sentence, or the AI's once it is there (now-page.md 5.1).
 * Both sit in the same grid cell and cross-fade, so the line keeps its height until the AI text
 * needs more. While the run works, a thin AI line glides under it; the mark, the time tooltip and
 * the refresh button appear with the finished text. If the first run fails, the preset
 * stays and a quiet retry button sits next to it.
 *
 * A refresh keeps the old sentence on screen (with the working line under it) and swaps in the new
 * one at the end with a cross-fade, so the line neither blanks nor jumps while text streams in.
 * A sentence that is already finished when it appears (a cached answer) gets one sheen of the AI
 * gradient across it; one that streamed in does not, and nothing runs with reduced motion.
 */
export function SummaryLine({ client, preset, summary, mode }: { client: MailxClient; preset: ReactNode; summary: NowSummary; mode: EntranceMode }) {
  useTranslation()
  const reduced = usePrefersReducedMotion()
  const [asking, setAsking] = useState(false)
  const { text, working, refreshing, completedAt, failed, needsConsent, service, refresh } = summary
  const ai = text !== ''
  const done = completedAt !== null

  // A finished sentence replaced by another one: the old fades out over the new one for a moment.
  // Decided while rendering, so the new text and its fade-in arrive in the same frame.
  const [seenDone, setSeenDone] = useState('')
  const [outgoing, setOutgoing] = useState<string | null>(null)
  if (done && text !== seenDone) {
    setSeenDone(text)
    if (seenDone !== '' && !reduced) setOutgoing(seenDone)
  }
  useEffect(() => {
    if (outgoing === null) return
    const timer = setTimeout(() => setOutgoing(null), SWAP_MS)
    return () => clearTimeout(timer)
  }, [outgoing])

  // One sheen when a finished sentence shows up without having streamed in this visit.
  const streamed = useRef(false)
  const sheened = useRef(false)
  const [sheen, setSheen] = useState(false)
  useEffect(() => {
    if (working) streamed.current = true
    if (!done || sheened.current) return
    sheened.current = true
    if (!streamed.current && !reduced && mode !== 'none') setSheen(true)
  }, [working, done, reduced, mode])

  const fade = reduced ? 'transition-opacity duration-100' : 'transition-opacity duration-200 ease-[cubic-bezier(0.2,0,0,1)]'
  return (
    <div data-slot="now-summary" className="relative mt-2 flex flex-wrap items-baseline gap-x-3">
      <div className="relative grid min-w-0">
        <p data-slot="now-summary-preset" aria-hidden={ai || undefined} className={cn('col-start-1 row-start-1 min-h-6 text-base text-muted-foreground', fade, ai && 'opacity-0')}>{preset}</p>
        {outgoing !== null && (
          <p aria-hidden data-slot="now-summary-outgoing" className="animate-now-text-out pointer-events-none col-start-1 row-start-1 pl-5 text-base text-muted-foreground">{outgoing}</p>
        )}
        <div data-slot="now-summary-ai" data-shown={ai} className={cn('col-start-1 row-start-1 flex items-start gap-1.5 text-base text-muted-foreground', fade, !ai && 'pointer-events-none opacity-0')}>
          {ai && (done
            ? <span className="mt-[5px] inline-flex"><AiGeneratedMark at={completedAt} /></span>
            : <AiMark className="mt-[5px]" />)}
          <span className={cn('relative min-w-0', outgoing !== null && 'animate-now-text-in')}>
            <span aria-hidden>{text}</span>
            {sheen && (
              <span
                aria-hidden
                data-slot="now-summary-sheen"
                onAnimationEnd={() => setSheen(false)}
                className="animate-ai-sheen pointer-events-none absolute opacity-70 inset-0 bg-clip-text text-transparent select-none [animation-delay:120ms]"
                style={{ backgroundImage: SHEEN_BACKGROUND, backgroundSize: '250% 100%' }}
              >
                {text}
              </span>
            )}
            {done && (
              <IconButton label={t('now:aiRefresh')} size="icon-xs" onClick={refresh} disabled={refreshing}
                className="ml-1 align-middle text-muted-foreground/60 hover:text-foreground focus-visible:text-foreground">
                <HugeiconsIcon icon={RefreshIcon} strokeWidth={2} className={cn(refreshing && 'motion-safe:animate-spin [animation-duration:2s]')} />
              </IconButton>
            )}
          </span>
          {/* Read out once, when it is complete, not word by word. */}
          <span role="status" className="sr-only">{done ? text : ''}</span>
        </div>
        <AiWorkingLine active={working} className="-bottom-1.5" />
      </div>
      {failed && (
        <IconButton label={t('now:aiRetry')} size="icon-xs" onClick={refresh}
          className="text-muted-foreground/60 hover:text-foreground focus-visible:text-foreground">
          <HugeiconsIcon icon={RefreshIcon} strokeWidth={2} />
        </IconButton>
      )}
      {needsConsent && (
        <button type="button" onClick={() => setAsking(true)}
          className="inline-flex items-center gap-1.5 rounded-md px-1.5 py-0.5 text-sm text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
        >
          <AiMark />{t('now:aiSummarize')}
        </button>
      )}
      <AiConsentDialog client={client} service={service} open={asking} onOpenChange={setAsking} onGranted={refresh} />
    </div>
  )
}
