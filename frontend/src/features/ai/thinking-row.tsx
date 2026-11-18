import { useEffect, useId, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { ArrowDown01Icon } from '@hugeicons/core-free-icons'
import { t } from '@/i18n'
import { usePrefersReducedMotion } from '@/hooks/use-reduced-motion'
import { cn } from '@/lib/utils'
import type { RunThought } from './use-assistant-run'

/** Whole seconds since `since`, ticking once a second; `null` stops the clock. */
function useSeconds(since: number | null) {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    if (since === null) return
    const timer = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(timer)
  }, [since])
  return since === null ? 0 : Math.max(0, Math.floor((now - since) / 1000))
}

/** "思考了 12 秒", or "思考了片刻" under a second. */
function doneLabel(durationMs: number) {
  return durationMs < 1000 ? t('ai:chat.reasoningBrief') : t('ai:chat.reasoningDone', { count: Math.round(durationMs / 1000) })
}

/**
 * One stretch of the model's thinking, as a quiet line in the tool-row style (ai-chat-ui.md 4.2). While
 * it goes: "思考中 · 12 秒" with the thinking sheen on the words. Once done: "思考了 12 秒", a button
 * that opens the raw reasoning below it (closed by default, also in saved replies). Redacted or empty
 * reasoning has nothing to open. The panel fades and grows once; under reduced motion it just appears.
 */
export function ThinkingRow({ thought, working }: { thought: RunThought; working: boolean }) {
  useTranslation()
  const reduced = usePrefersReducedMotion()
  const [open, setOpen] = useState(false)
  const panel = useId()
  const going = working && thought.durationMs === null
  const seconds = useSeconds(going ? thought.startedAt : null)
  const readable = !going && thought.text.trim() !== ''
  const words = going
    ? (
      <>
        <span className="ai-shimmer">{t('ai:chat.reasoningLive')}</span>
        {seconds > 0 && <span aria-hidden> · {t('ai:chat.reasoningSeconds', { count: seconds })}</span>}
      </>
    )
    : doneLabel(thought.durationMs ?? 0)
  return (
    <div data-slot="thinking-row" data-state={going ? 'live' : 'done'} className="flex flex-col text-sm text-muted-foreground">
      <div className="flex min-h-6 items-center gap-x-1.5">
        {/* An empty slot the size of a tool row's icon: the words line up, and thinking (not an action) has no mark of its own. */}
        <span aria-hidden className="size-3.5 shrink-0" />
        {readable
          ? (
            <button
              type="button"
              aria-expanded={open}
              aria-controls={panel}
              title={open ? t('ai:chat.hideReasoning') : t('ai:chat.showReasoning')}
              onClick={() => setOpen((value) => !value)}
              className="flex items-center gap-x-1.5 rounded-sm outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
            >
              {/* The words change in place with a short fade; the line keeps its height. */}
              <span key="done" className="animate-now-text-in">{words}</span>
              <HugeiconsIcon icon={ArrowDown01Icon} aria-hidden strokeWidth={2} className={cn('size-3.5 transition-transform duration-150 motion-reduce:transition-none', open && 'rotate-180')} />
            </button>
          )
          : <span key={going ? 'live' : 'done'} className="animate-now-text-in">{words}</span>}
      </div>
      {readable && (
        <div
          id={panel}
          data-slot="thinking-panel"
          data-open={open}
          inert={!open}
          className={cn(
            'grid',
            reduced ? 'transition-none' : 'transition-[grid-template-rows,opacity] duration-200 ease-out',
            open ? 'grid-rows-[1fr] opacity-100' : 'grid-rows-[0fr] opacity-0',
          )}
        >
          <div className="min-h-0 overflow-hidden">
            <p className="mt-1.5 mb-1 ml-5 border-l-2 border-foreground/15 pl-3 text-xs leading-relaxed wrap-anywhere whitespace-pre-wrap">
              {thought.text}
            </p>
          </div>
        </div>
      )}
    </div>
  )
}
