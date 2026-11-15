import { useCallback, useEffect, useRef, useState } from 'react'
import type { MailxClient } from '@/lib/mailx-client'
import { assistantTarget } from '@/features/ai/assistant-target'
import { useAssistantRun } from '@/features/ai/use-assistant-run'
import { useAiState } from '@/features/settings/ai-queries'
import { useSettings } from '@/features/settings/settings-queries'

export type NowSummary = {
  /**
   * The AI sentence to show in place of the preset line; empty while there is none (or the first run
   * failed). Once a sentence has finished it stays until a refresh finishes with a new one: a running
   * or failed refresh never blanks it.
   */
  text: string
  /** A run is going: show the working treatment. */
  working: boolean
  /** A refresh is going while the previous sentence is still shown. */
  refreshing: boolean
  /** The shown sentence is finished; the host's completion time (a cached answer keeps its original one). */
  completedAt: Date | null
  /** The assistant is ready but has no consent yet: offer "用 AI 总结". */
  needsConsent: boolean
  /** The connection the mail would go to, for the consent dialog. */
  service: string
  /** Asks for a new sentence, skipping the host's cached one. */
  refresh: () => void
}

/**
 * The AI sentence on the Now page (now-page.md 5.1). When the assistant is on and a model is usable,
 * one `nowSummary` run starts once the overview is in, with the same `since` and `dayStarts` the page
 * uses. A missing consent leaves the preset line and an offer; any failure leaves the preset line and
 * says nothing. Leaving the page cancels the run (the run hook does). Only a refresh starts another,
 * and it asks the host to skip its cache. The finished sentence is kept here, because the run hook
 * clears its text whenever it starts a run.
 */
export function useNowSummary({ client, ready, since, dayStarts }: { client: MailxClient; ready: boolean; since: string; dayStarts: string[] }): NowSummary {
  const settings = useSettings(client)
  const assistant = settings.data?.settings.assistant
  const on = assistant?.enabled === true
  const ai = useAiState(client, on)
  const target = on ? assistantTarget(ai.data, assistant) : null
  const usable = target !== null
  const { start, state, text, finishedAt } = useAssistantRun(client)
  // What the page is showing now; a refresh asks about the same period, not a later one.
  const latest = useRef({ since, dayStarts })
  useEffect(() => { latest.current = { since, dayStarts } })
  const begin = useCallback((fresh: boolean) => {
    void start({ type: 'nowSummary', since: latest.current.since, dayStarts: latest.current.dayStarts, fresh })
  }, [start])
  const refresh = useCallback(() => begin(true), [begin])
  const started = useRef(false)
  useEffect(() => {
    if (!usable || !ready || started.current) return
    started.current = true
    begin(false)
  }, [usable, ready, begin])

  const live = usable && (state === 'completed' || state === 'running' || state === 'awaitingConfirmation') ? text.trim() : ''
  const running = usable && (state === 'running' || state === 'awaitingConfirmation')
  const finished = usable && state === 'completed' && live !== ''
  // The last finished sentence and its time.
  const [kept, setKept] = useState<{ text: string; at: Date | null } | null>(null)
  if (finished && (kept?.text !== live || kept.at !== finishedAt)) setKept({ text: live, at: finishedAt })
  const holding = usable && kept !== null
  const busy = usable && (state === 'starting' || running)
  return {
    // A refresh changes the line only when it completes (see SummaryLine); until then the kept sentence stays.
    text: finished ? live : holding ? kept.text : live,
    working: running || (holding && busy),
    refreshing: holding && busy,
    completedAt: finished ? finishedAt : holding ? kept.at : null,
    needsConsent: usable && state === 'awaitingConsent',
    service: target?.connection.name ?? '',
    refresh,
  }
}
