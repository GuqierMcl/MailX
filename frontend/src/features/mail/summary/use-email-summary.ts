import { useEffect, useEffectEvent, useRef, useState } from 'react'
import type { CachedEmailSummary, EmailDetail, ErrorCode, FinishedReason, SummaryLanguage, SummaryStyle } from '@/generated/accounts'
import { MailxClientError, type MailxClient } from '@/lib/mailx-client'
import { useAssistantRun } from '@/features/ai/use-assistant-run'
import { useAssistantUsable } from '@/features/ai/use-assistant-usable'
import { DEFAULT_EMAIL_SUMMARY, useSettings, useUpdateAssistant } from '@/features/settings/settings-queries'

/** Body text shorter than this is not summarized on its own; asking still works. */
export const AUTO_SUMMARY_MIN_CHARS = 200

export type SummaryPhase = 'idle' | 'working' | 'done' | 'consent' | 'error'

export type EmailSummaryState = {
  /** The assistant can summarize this email: it is on, has a model, sees the account, and the body is here. */
  available: boolean
  phase: SummaryPhase
  /** What the model wrote so far, or the kept summary. */
  text: string
  /** The style of what `text` shows (or is being written in). */
  style: SummaryStyle | null
  /** The output cap cut it off: shown as it is, not kept. */
  cut: boolean
  error: ErrorCode | null
  /** When the shown summary was made. */
  at: Date | null
  /** The card is on screen (a summary exists or is being made, and it was not closed). */
  shown: boolean
  collapsed: boolean
  /** The panel's choices, as saved. */
  preferredStyle: SummaryStyle
  preferredLanguage: SummaryLanguage
  /** Mail from this sender is always summarized. */
  alwaysForSender: boolean
  senderAddress: string
  /** The connection the mail would go to, for the consent dialog. */
  service: string
  /** Makes (or shows the kept) summary in this style and language and saves them as the panel's choice. */
  generate: (style: SummaryStyle, language: SummaryLanguage) => void
  /** Asks the model again in the shown style and language. */
  regenerate: () => void
  /** Retries after consent was given. */
  resume: () => void
  stop: () => void
  /** Hides the card for this email; the kept summary stays. */
  dismiss: () => void
  toggleCollapsed: () => void
  setAlwaysForSender: (always: boolean) => void
}

type Request = { style: SummaryStyle; language: SummaryLanguage; fresh: boolean }
type Kept = { text: string; style: SummaryStyle | null; at: Date | null }

const failureCode = (error: unknown): ErrorCode => {
  const code = error instanceof MailxClientError ? error.failure.code : null
  return code && code !== 'hostUnavailable' && code !== 'transport' ? code : 'internal'
}

/**
 * The AI summary of the opened email (ai-chat-ui.md 5.2). On opening, a summary already kept on this
 * device shows at once; otherwise one is made when the user turned automatic summaries on (for bodies of
 * at least `AUTO_SUMMARY_MIN_CHARS`) or always summarizes this sender. Everything else waits for the
 * panel. Switching to another email stops a summary still being written.
 */
export function useEmailSummary(client: MailxClient, email: EmailDetail | null): EmailSummaryState {
  const settings = useSettings(client)
  const assistant = settings.data?.settings.assistant
  const preferences = assistant?.summary ?? DEFAULT_EMAIL_SUMMARY
  const update = useUpdateAssistant(client)
  const { usable, service } = useAssistantUsable(client)
  const run = useAssistantRun(client)
  const { start, cancel } = run
  const emailId = email?.summary.id ?? null
  const accountId = email?.summary.accountId ?? null
  const senderAddress = email?.summary.from.address.trim().toLowerCase() ?? ''
  const bodyLength = email?.body?.text?.trim().length ?? 0
  const hasBody = Boolean(email?.body)
  const excluded = accountId !== null && (assistant?.excludedAccounts ?? []).includes(accountId)
  const available = usable && hasBody && !excluded && emailId !== null

  const [kept, setKept] = useState<Kept | null>(null)
  const [shown, setShown] = useState(false)
  const [startError, setStartError] = useState<ErrorCode | null>(null)
  /** The last request made for this email; `null` until one was. */
  const [requested, setRequested] = useState<Request | null>(null)
  const current = useRef(emailId)
  useEffect(() => { current.current = emailId }, [emailId])

  const begin = (request: Request) => {
    if (!emailId) return
    setRequested(request)
    setShown(true)
    setStartError(null)
    start({ type: 'emailSummary', emailId, style: request.style, language: request.language, fresh: request.fresh }).catch(
      (error: unknown) => { if (current.current === emailId) setStartError(failureCode(error)) },
    )
  }

  // A new email: forget the old one's card and stop what was being written for it.
  const [seen, setSeen] = useState(emailId)
  if (seen !== emailId) {
    setSeen(emailId)
    setKept(null)
    setShown(false)
    setStartError(null)
    setRequested(null)
  }
  useEffect(() => () => cancel(), [emailId, cancel])

  // On opening: the kept summary, else the automatic one. Once per email, when it becomes available.
  const opened = useRef<string | null>(null)
  const auto = preferences.auto
  const always = senderAddress !== '' && preferences.senders.includes(senderAddress)
  const style = preferences.style
  const language = preferences.language
  // Reads the latest choices without making the lookup run again when they change.
  const onFound = useEffectEvent((id: string, found: CachedEmailSummary | null) => {
    if (current.current !== id) return
    if (found) {
      setKept({ text: found.text, style: found.style, at: new Date(found.createdAt) })
      setShown(true)
    } else if (always || (auto && bodyLength >= AUTO_SUMMARY_MIN_CHARS)) {
      begin({ style, language, fresh: false })
    }
  })
  useEffect(() => {
    if (!available || !emailId || opened.current === emailId) return
    opened.current = emailId
    let alive = true
    client.assistantEmailSummary(emailId).then(
      (found) => { if (alive) onFound(emailId, found) },
      () => undefined,
    )
    return () => { alive = false }
  }, [available, emailId, client])

  const save = (change: Partial<typeof preferences>) => {
    update.mutate({ summary: { ...preferences, ...change } })
  }

  const generate = (nextStyle: SummaryStyle, nextLanguage: SummaryLanguage) => {
    if (nextStyle !== preferences.style || JSON.stringify(nextLanguage) !== JSON.stringify(preferences.language)) {
      save({ style: nextStyle, language: nextLanguage })
    }
    begin({ style: nextStyle, language: nextLanguage, fresh: false })
  }
  const regenerate = () => {
    begin({ style: requested?.style ?? kept?.style ?? style, language: requested?.language ?? language, fresh: true })
  }
  const resume = () => {
    if (requested) begin({ ...requested })
  }
  const dismiss = () => {
    cancel()
    setShown(false)
  }
  const toggleCollapsed = () => save({ collapsed: !preferences.collapsed })
  const setAlwaysForSender = (on: boolean) => {
    if (!senderAddress) return
    const rest = preferences.senders.filter((address) => address !== senderAddress)
    save({ senders: on ? [...rest, senderAddress] : rest })
  }

  // What the run says, once a run was started for this email.
  const runState = requested ? run.state : 'idle'
  const runText = run.text.trim()
  let phase: SummaryPhase = kept ? 'done' : 'idle'
  let text = kept?.text ?? ''
  let at = kept?.at ?? null
  let shownStyle = kept?.style ?? null
  let error: ErrorCode | null = startError
  const finished: FinishedReason | null = run.finishedReason
  if (startError) phase = 'error'
  else if (runState === 'starting' || runState === 'running') {
    phase = 'working'
    text = runText
    shownStyle = requested?.style ?? null
    at = null
  } else if (runState === 'awaitingConsent') phase = 'consent'
  else if (runState === 'completed' && runText) {
    phase = 'done'
    text = runText
    shownStyle = requested?.style ?? null
    at = run.finishedAt
  } else if (runState === 'failed') {
    phase = 'error'
    error = run.error ?? 'internal'
  } else if (runState === 'canceled' && !kept) {
    phase = runText ? 'done' : 'idle'
    text = runText
    shownStyle = requested?.style ?? null
  }

  return {
    available,
    phase,
    text,
    style: shownStyle,
    cut: phase === 'done' && runState === 'completed' && finished === 'length',
    error,
    at,
    shown: available && shown && phase !== 'idle',
    collapsed: preferences.collapsed,
    preferredStyle: preferences.style,
    preferredLanguage: preferences.language,
    alwaysForSender: always,
    senderAddress,
    service,
    generate,
    regenerate,
    resume,
    stop: cancel,
    dismiss,
    toggleCollapsed,
    setAlwaysForSender,
  }
}
