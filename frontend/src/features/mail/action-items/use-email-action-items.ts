import { useCallback, useEffect, useEffectEvent, useMemo, useRef, useState } from 'react'
import type {
  ActionItemStatus,
  CachedEmailActionItems,
  DetectedActionItem,
  EmailDetail,
  ErrorCode,
} from '@/generated/accounts'
import { MailxClientError, type MailxClient } from '@/lib/mailx-client'
import { useAssistantRun } from '@/features/ai/use-assistant-run'
import { useAssistantUsable } from '@/features/ai/use-assistant-usable'
import { DEFAULT_ACTION_ITEMS, useSettings, useUpdateAssistant } from '@/features/settings/settings-queries'

export type ActionItemsPhase = 'idle' | 'working' | 'done' | 'consent' | 'error'

const AUTO_EXTRACT_DEBOUNCE_MS = 1000

const failureCode = (error: unknown): ErrorCode => {
  const code = error instanceof MailxClientError ? error.failure.code : null
  return code && code !== 'hostUnavailable' && code !== 'transport' ? code : 'internal'
}

export function localTimeZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || 'UTC'
  } catch {
    return 'UTC'
  }
}

function sameDatePrefix(a: string | null, b: string | null): boolean {
  if (!a || !b || a.length < 10 || b.length < 10) return false
  return a.slice(0, 10) === b.slice(0, 10)
}

export type EmailActionItemsState = {
  /** Whether L2 AI extraction can run for this email (assistant on, model configured, account not excluded, body cached, not local-only). */
  aiAvailable: boolean
  phase: ActionItemsPhase
  error: ErrorCode | null
  service: string
  /** L0 `.ics` events from the email. */
  icsItems: DetectedActionItem[]
  /** L2 AI-extracted events and tasks. */
  aiItems: DetectedActionItem[]
  /** L1 locally detected items in the current original body document. */
  l1Items: DetectedActionItem[]
  /** Combined L0 (first) + L2 items, excluding dismissed ones. */
  visibleItems: DetectedActionItem[]
  /** Whether to show the first-hit guidance banner above the body. */
  showPromptBanner: boolean
  /** True when a manual AI extraction finished with zero items and the user has not dismissed the notice. */
  showEmptyNotice: boolean
  /** Whether automatic AI extraction on L1 hit is enabled in settings. */
  autoExtract: boolean
  /** Currently open item in the unified edit/confirm dialog. */
  editingItem: DetectedActionItem | null
  /** Report L1 items detected in the active original document frame. */
  setL1Items: (items: DetectedActionItem[]) => void
  /** Open the unified confirm dialog for any L0/L1/L2 item (enriches L1 with matching L2 data when available). */
  openItem: (item: DetectedActionItem) => void
  closeDialog: () => void
  extract: (fresh?: boolean) => void
  stop: () => void
  resume: () => void
  dismissEmptyNotice: () => void
  setAutoExtract: (enabled: boolean) => void
  enableAutoFromBanner: () => void
  dismissPromptForever: () => void
  closePromptForSession: () => void
  markStatus: (itemId: string, status: ActionItemStatus) => Promise<void>
}

export function useEmailActionItems(
  client: MailxClient,
  email: EmailDetail | null,
  summaryWorking = false,
): EmailActionItemsState {
  const settings = useSettings(client)
  const assistant = settings.data?.settings.assistant
  const prefs = assistant?.actionItems ?? DEFAULT_ACTION_ITEMS
  const updateAssistant = useUpdateAssistant(client)
  const { usable, service } = useAssistantUsable(client)
  const run = useAssistantRun(client)
  const { start, cancel } = run

  const emailId = email?.summary.id ?? null
  const accountId = email?.summary.accountId ?? null
  const localOnly = email?.summary.localOnly ?? false
  const hasBody = Boolean(email?.body)
  const excluded = accountId !== null && (assistant?.excludedAccounts ?? []).includes(accountId)
  const aiAvailable = usable && hasBody && !excluded && !localOnly && emailId !== null
  const timeZone = useMemo(() => localTimeZone(), [])

  const [icsItems, setIcsItems] = useState<DetectedActionItem[]>([])
  const [aiItems, setAiItems] = useState<DetectedActionItem[]>([])
  const [aiLoaded, setAiLoaded] = useState(false)
  const [l1Items, setL1ItemsState] = useState<DetectedActionItem[]>([])
  const [editingItem, setEditingItem] = useState<DetectedActionItem | null>(null)
  const [sessionPromptClosed, setSessionPromptClosed] = useState(false)
  const [requestedFresh, setRequestedFresh] = useState<boolean | null>(null)
  const [manualRequested, setManualRequested] = useState(false)
  const [emptyDismissed, setEmptyDismissed] = useState(false)
  const [startError, setStartError] = useState<ErrorCode | null>(null)

  const currentEmailId = useRef(emailId)
  useEffect(() => { currentEmailId.current = emailId }, [emailId])

  // Reset per-email state on switch and cancel any in-flight AI extraction.
  const [seenEmailId, setSeenEmailId] = useState(emailId)
  if (seenEmailId !== emailId) {
    setSeenEmailId(emailId)
    setIcsItems([])
    setAiItems([])
    setAiLoaded(false)
    setL1ItemsState([])
    setEditingItem(null)
    setRequestedFresh(null)
    setManualRequested(false)
    setEmptyDismissed(false)
    setStartError(null)
  }
  useEffect(() => () => cancel(), [emailId, cancel])

  // Load L0 `.ics` items when body is available.
  useEffect(() => {
    if (!emailId || !hasBody || localOnly) return
    let alive = true
    client.getCalendarItems(emailId).then(
      (items) => { if (alive && currentEmailId.current === emailId) setIcsItems(items) },
      () => undefined,
    )
    return () => { alive = false }
  }, [client, emailId, hasBody, localOnly])

  // Load cached L2 AI items on open (local cache only, no consent needed).
  const loadCachedAi = useCallback((id: string) => {
    client.assistantEmailActionItems(id, timeZone).then(
      (cached: CachedEmailActionItems | null) => {
        if (currentEmailId.current !== id) return
        setAiLoaded(true)
        if (cached) setAiItems(cached.items)
      },
      () => {
        if (currentEmailId.current === id) setAiLoaded(true)
      },
    )
  }, [client, timeZone])

  useEffect(() => {
    if (!aiAvailable || !emailId) return
    loadCachedAi(emailId)
  }, [aiAvailable, emailId, loadCachedAi])

  const beginExtract = useCallback((fresh: boolean, manual: boolean) => {
    if (!emailId) return
    setRequestedFresh(fresh)
    if (manual) {
      setManualRequested(true)
      setEmptyDismissed(false)
    }
    setStartError(null)
    start({ type: 'extractActionItems', emailId, timeZone, fresh }).catch(
      (error: unknown) => {
        if (currentEmailId.current === emailId) setStartError(failureCode(error))
      },
    )
  }, [emailId, start, timeZone])

  // Reload cached L2 items whenever a distinct AI extraction run completes (including cache hits).
  const completedRunId = useRef<string | null>(null)
  useEffect(() => {
    if (requestedFresh !== null && run.state === 'completed' && run.runId && completedRunId.current !== run.runId && emailId) {
      completedRunId.current = run.runId
      loadCachedAi(emailId)
    }
  }, [run.state, run.runId, requestedFresh, emailId, loadCachedAi])

  // Auto-extract with 1s dwell debounce when L1 hits, after summary is not actively working.
  const autoTriggeredFor = useRef<string | null>(null)
  const triggerAuto = useEffectEvent(() => {
    if (!emailId || !aiAvailable || !prefs.autoExtract || summaryWorking) return
    if (l1Items.length === 0 || aiItems.length > 0 || requestedFresh !== null) return
    if (autoTriggeredFor.current === emailId) return
    autoTriggeredFor.current = emailId
    beginExtract(false, false)
  })
  useEffect(() => {
    if (!emailId || !aiAvailable || !prefs.autoExtract || !aiLoaded || summaryWorking) return
    if (l1Items.length === 0 || aiItems.length > 0 || requestedFresh !== null) return
    if (autoTriggeredFor.current === emailId) return
    const timer = window.setTimeout(() => triggerAuto(), AUTO_EXTRACT_DEBOUNCE_MS)
    return () => window.clearTimeout(timer)
  }, [emailId, aiAvailable, prefs.autoExtract, aiLoaded, summaryWorking, l1Items.length, aiItems.length, requestedFresh])

  const savePrefs = (patch: Partial<typeof prefs>) => {
    updateAssistant.mutate({ actionItems: { ...prefs, ...patch } })
  }

  const openItem = useCallback((item: DetectedActionItem) => {
    if (item.source === 'rule' && aiItems.length > 0 && item.startAt) {
      const matchedL2 = aiItems.find(
        (candidate) => candidate.status !== 'dismissed' && sameDatePrefix(candidate.startAt, item.startAt),
      )
      if (matchedL2) {
        setEditingItem({
          ...item,
          title: matchedL2.title || item.title,
          location: matchedL2.location ?? item.location,
          endAt: matchedL2.endAt ?? item.endAt,
          url: matchedL2.url ?? item.url,
          notes: matchedL2.notes ?? item.notes,
        })
        return
      }
    }
    setEditingItem(item)
  }, [aiItems])

  const markStatus = useCallback(async (itemId: string, status: ActionItemStatus) => {
    if (!emailId) return
    const apply = (list: DetectedActionItem[]) =>
      list.map((item) => (item.id === itemId ? { ...item, status } : item))
    setIcsItems(apply)
    setAiItems(apply)
    setL1ItemsState(apply)
    await client.setActionItemStatus(emailId, itemId, status)
  }, [client, emailId])

  const runState = requestedFresh !== null ? run.state : 'idle'
  let phase: ActionItemsPhase = aiItems.length > 0 ? 'done' : 'idle'
  let error: ErrorCode | null = startError
  if (startError) phase = 'error'
  else if (runState === 'starting' || runState === 'running') phase = 'working'
  else if (runState === 'awaitingConsent') phase = 'consent'
  else if (runState === 'completed') phase = 'done'
  else if (runState === 'failed') {
    phase = 'error'
    error = run.error ?? 'internal'
  }

  const visibleItems = useMemo(() => {
    const ics = icsItems.filter((item) => item.status !== 'dismissed')
    const ai = aiItems.filter((item) => item.status !== 'dismissed')
    return [...ics, ...ai]
  }, [icsItems, aiItems])

  const showPromptBanner =
    aiAvailable &&
    !prefs.autoExtract &&
    !prefs.promptDismissed &&
    !sessionPromptClosed &&
    l1Items.length > 0 &&
    phase === 'idle'

  const showEmptyNotice =
    manualRequested &&
    !emptyDismissed &&
    phase === 'done' &&
    aiLoaded &&
    aiItems.length === 0

  return {
    aiAvailable,
    phase,
    error,
    service,
    icsItems,
    aiItems,
    l1Items,
    visibleItems,
    showPromptBanner,
    showEmptyNotice,
    autoExtract: prefs.autoExtract,
    editingItem,
    setL1Items: setL1ItemsState,
    openItem,
    closeDialog: () => setEditingItem(null),
    extract: (fresh = false) => beginExtract(fresh, true),
    stop: cancel,
    resume: () => beginExtract(requestedFresh ?? false, manualRequested),
    dismissEmptyNotice: () => setEmptyDismissed(true),
    setAutoExtract: (enabled) => savePrefs({ autoExtract: enabled }),
    enableAutoFromBanner: () => {
      savePrefs({ autoExtract: true })
      beginExtract(false, true)
    },
    dismissPromptForever: () => savePrefs({ promptDismissed: true }),
    closePromptForSession: () => setSessionPromptClosed(true),
    markStatus,
  }
}
