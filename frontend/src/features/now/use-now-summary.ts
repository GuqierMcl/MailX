import { useCallback, useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { MailxClient } from '@/lib/mailx-client'
import { assistantTarget } from '@/features/ai/assistant-target'
import { useAssistantRun } from '@/features/ai/use-assistant-run'
import { useAiState } from '@/features/settings/ai-queries'
import { useSettings } from '@/features/settings/settings-queries'
import { occasionOn } from './calendar'

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
  /**
   * The first run failed and no sentence is kept, so only the preset line shows: offer a retry. Not
   * set while a run is going, when a sentence is kept (a failed refresh leaves it as it was), when
   * consent is missing, or when the assistant is off or has no model (retrying cannot help there).
   */
  failed: boolean
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
 * uses. A missing consent leaves the preset line and an offer; a failure leaves the preset line, with a quiet
 * retry button when no earlier sentence is kept. Leaving the page cancels the run (the run hook does). Only a
 * refresh (or that retry) starts another, and it asks the host to skip its cache. The finished sentence is kept here, because the run hook
 * clears its text whenever it starts a run.
 */
export function useNowSummary({ client, ready, since, dayStarts }: { client: MailxClient; ready: boolean; since: string; dayStarts: string[] }): NowSummary {
  const settings = useSettings(client)
  const assistant = settings.data?.settings.assistant
  const on = assistant?.enabled === true
  const ai = useAiState(client, on)
  const target = on ? assistantTarget(ai.data, assistant) : null
  const usable = target !== null
  const { start, state, text, finishedAt, error } = useAssistantRun(client)
  const { i18n } = useTranslation()
  // What the page is showing now; a refresh asks about the same period, not a later one.
  const latest = useRef({ since, dayStarts, language: i18n.language })
  useEffect(() => { latest.current = { since, dayStarts, language: i18n.language } })
  const begin = useCallback((fresh: boolean) => {
    const { since, dayStarts, language } = latest.current
    // What today is, by the same rules as the preset line. A closed id the host turns into a fact, never text.
    const found = occasionOn(new Date(), language)
    const occasion = found ? (found.kind === 'holiday' ? found.holiday : found.term) : undefined
    void start({ type: 'nowSummary', since, dayStarts, fresh, ...(occasion ? { occasion } : {}) })
  }, [start])
  const refresh = useCallback(() => begin(true), [begin])
  const started = useRef(false)
  useEffect(() => {
    if (!usable || !ready || started.current) return
    started.current = true
    begin(false)
  }, [usable, ready, begin])
  // Consent given elsewhere (the chat, another window) while this line waits for it: try again once.
  const consented = assistant?.consented ?? null
  const waitingFor = useRef<typeof consented | undefined>(undefined)
  useEffect(() => {
    if (state !== 'awaitingConsent') { waitingFor.current = undefined; return }
    if (waitingFor.current === undefined) { waitingFor.current = consented; return }
    if (consented && consented !== waitingFor.current) {
      waitingFor.current = consented
      begin(false)
    }
  }, [state, consented, begin])

  const live = usable && (state === 'completed' || state === 'running' || state === 'awaitingConfirmation') ? text.trim() : ''
  const running = usable && (state === 'running' || state === 'awaitingConfirmation')
  const finished = usable && state === 'completed' && live !== ''
  // The last finished sentence and its time.
  const [kept, setKept] = useState<{ text: string; at: Date | null } | null>(null)
  if (finished && (kept?.text !== live || kept.at !== finishedAt)) setKept({ text: live, at: finishedAt })
  const holding = usable && kept !== null
  // A refused start (assistant off, no model) is not worth a retry.
  const failed = usable && !holding && state === 'failed' && error !== 'assistantDisabled' && error !== 'aiNotConfigured'
  const busy = usable && (state === 'starting' || running)
  return {
    // A refresh changes the line only when it completes (see SummaryLine); until then the kept sentence stays.
    text: finished ? live : holding ? kept.text : live,
    working: running || (holding && busy),
    refreshing: holding && busy,
    completedAt: finished ? finishedAt : holding ? kept.at : null,
    failed,
    needsConsent: usable && state === 'awaitingConsent',
    service: target?.connection.name ?? '',
    refresh,
  }
}
