import { useCallback, useEffect, useRef } from 'react'
import type { MailxClient } from '@/lib/mailx-client'
import { assistantTarget } from '@/features/ai/assistant-target'
import { useAssistantRun } from '@/features/ai/use-assistant-run'
import { useAiState } from '@/features/settings/ai-queries'
import { useSettings } from '@/features/settings/settings-queries'

export type NowSummary = {
  /** The AI sentence to show in place of the preset line; empty while there is none (or it failed). */
  text: string
  /** A run is going: show the working treatment. */
  working: boolean
  /** The run finished with text; the host's completion time (a cached answer keeps its original one). */
  completedAt: Date | null
  /** The assistant is ready but has no consent yet: offer "用 AI 总结". */
  needsConsent: boolean
  /** The connection the mail would go to, for the consent dialog. */
  service: string
  /** Starts a new run, replacing the current one. */
  refresh: () => void
}

/**
 * The AI sentence on the Now page (now-page.md 5.1). When the assistant is on and a model is usable,
 * one `nowSummary` run starts once the overview is in, with the same `since` and `dayStarts` the page
 * uses. A missing consent leaves the preset line and an offer; any failure leaves the preset line and
 * says nothing. Leaving the page cancels the run (the run hook does). Only a refresh starts another.
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
  const begin = useCallback(() => {
    void start({ type: 'nowSummary', since: latest.current.since, dayStarts: latest.current.dayStarts })
  }, [start])
  const started = useRef(false)
  useEffect(() => {
    if (!usable || !ready || started.current) return
    started.current = true
    begin()
  }, [usable, ready, begin])

  const shown = usable && (state === 'completed' || state === 'running' || state === 'awaitingConfirmation') ? text.trim() : ''
  return {
    text: shown,
    working: usable && (state === 'running' || state === 'awaitingConfirmation'),
    completedAt: usable && state === 'completed' && shown !== '' ? finishedAt : null,
    needsConsent: usable && state === 'awaitingConsent',
    service: target?.connection.name ?? '',
    refresh: begin,
  }
}
