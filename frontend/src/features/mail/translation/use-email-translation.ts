import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { useQuery } from '@tanstack/react-query'
import type { EmailDetail, TranslationSettings, TranslationStatus } from '@/generated/accounts'
import { useSettings, useUpdateTranslation } from '@/features/settings/settings-queries'
import { clientFailureCode, type MailxClient } from '@/lib/mailx-client'
import { translationKeys } from '@/lib/query-client'
import { needsKey, TRANSLATION_PROVIDER_NAMES } from './languages'

/** How many messages the session remembers a choice for; the oldest are forgotten first. */
const MEMORY_LIMIT = 200

type Showing = 'original' | 'translation'
export type TranslationPhase = 'idle' | 'translating' | 'translated' | 'error'

/** What the user chose for one message earlier in this app session. Never persisted. */
type Remembered = { mode?: Showing; dismissed?: boolean }

type State = {
  id: string
  status: TranslationStatus | null
  busy: boolean
  /** The failure that ended the last attempt; `translationCancelled` never lands here. */
  error: { failure: unknown } | null
  showing: Showing
  /** The translation has been on screen for this message (so "showing the original" is a choice, not the default). */
  viewed: boolean
  dismissed: boolean
  /** A language picked from the menu: the status call only knows the configured target. */
  override: { target: string; basis: string } | null
  /** Grows with every new translation so the translated document is prepared afresh. */
  generation: number
  /** Waiting for the user's consent before the first translation; `target` is what they asked for. */
  consent: { target: string | undefined } | null
}

const fresh = (id: string, memory: Map<string, Remembered>): State => {
  const remembered = memory.get(id)
  return {
    id, status: null, busy: false, error: null, showing: remembered?.mode ?? 'original', viewed: remembered?.mode !== undefined,
    dismissed: remembered?.dismissed ?? false, override: null, generation: 0, consent: null,
  }
}

/** Settings that change what the status call answers, or whether a translation can run. */
const basisOf = (settings: TranslationSettings) => `${settings.targetLanguage}|${settings.includeQuotes}`

/**
 * Translation of the opened message: one place that the toolbar, the notice bar,
 * the body, the consent dialog and the command palette all read. Everything that
 * happens to a message other than the opened one is ignored, and leaving a
 * message that is still translating cancels it.
 */
export function useEmailTranslation(client: MailxClient, email: EmailDetail | null, openSettings: () => void) {
  const settingsQuery = useSettings(client)
  const settings = settingsQuery.data?.settings.translation ?? null
  const keys = useQuery({ queryKey: translationKeys.keys(), queryFn: () => client.translationKeys() })
  const update = useUpdateTranslation(client)

  const id = email?.summary.id ?? ''
  // Device-only sent copies and messages without an HTML document have nothing to translate.
  const eligible = Boolean(email && !email.summary.localOnly && email.summary.bodyState === 'ready' && email.body?.hasHtml)
  const provider = settings?.provider ?? null
  // A keyless service is ready as it is; a keyed one needs its saved key.
  const keyless = provider !== null && !needsKey(provider)
  const configured = provider !== null && (keyless || Boolean(keys.data?.some((key) => key.provider === provider && key.configured)))

  const [memory] = useState(() => new Map<string, Remembered>())
  const [stored, setStored] = useState<State>(() => fresh(id, memory))
  const stateRef = useRef(stored)
  const idRef = useRef(id)
  const latest = useRef({ settings, configured })
  const runs = useRef(0)
  /** Moves whenever a translation starts, ends or is stopped, so a status answer asked for earlier can be told apart. */
  const epoch = useRef(0)
  useLayoutEffect(() => {
    idRef.current = id
    latest.current = { settings, configured }
  })
  // A message other than the one the state belongs to starts from scratch.
  const state = stored.id === id ? stored : fresh(id, memory)

  const patch = useCallback((forId: string, change: Partial<State> | ((state: State) => Partial<State>)) => {
    if (idRef.current !== forId) return
    const base = stateRef.current.id === forId ? stateRef.current : fresh(forId, memory)
    const next = { ...base, ...(typeof change === 'function' ? change(base) : change) }
    stateRef.current = next
    setStored(next)
  }, [memory])

  const remember = useCallback((forId: string, change: Remembered) => {
    const next = { ...memory.get(forId), ...change }
    memory.delete(forId)
    memory.set(forId, next)
    if (memory.size > MEMORY_LIMIT) memory.delete(memory.keys().next().value as string)
  }, [memory])

  const begin = useCallback((forId: string, target: string | undefined) => {
    const run = ++runs.current
    epoch.current += 1
    const basis = latest.current.settings ? basisOf(latest.current.settings) : ''
    patch(forId, { busy: true, error: null, consent: null })
    client.translateEmail(forId, target).then((status) => {
      if (runs.current !== run) return
      epoch.current += 1
      remember(forId, { mode: 'translation' })
      patch(forId, (current) => ({
        busy: false, status, showing: 'translation', viewed: true, generation: current.generation + 1,
        override: target ? { target: status.targetLanguage, basis } : null,
      }))
    }, (failure: unknown) => {
      if (runs.current !== run) return
      epoch.current += 1
      const code = clientFailureCode(failure)
      if (code === 'translationCancelled') patch(forId, { busy: false })
      else if (code === 'translationConsentRequired') patch(forId, { busy: false, consent: { target } })
      else patch(forId, { busy: false, error: { failure } })
    })
  }, [client, patch, remember])

  const translate = useCallback((target?: string) => {
    const forId = idRef.current
    if (!forId) return
    const { settings: current, configured: ready } = latest.current
    if (!current?.provider || !ready) {
      patch(forId, { error: { failure: { code: 'translationNotConfigured' } } })
    } else if (current.consentedProvider !== current.provider) patch(forId, { consent: { target } })
    else begin(forId, target)
  }, [begin, patch])

  const cancel = useCallback(() => {
    const forId = idRef.current
    if (!forId) return
    runs.current += 1
    epoch.current += 1
    patch(forId, { busy: false })
    void client.cancelTranslation(forId).catch(() => undefined)
  }, [client, patch])

  // Leaving a message that is still translating stops the work.
  useEffect(() => {
    const leaving = id
    return () => {
      const current = stateRef.current
      if (leaving && current.id === leaving && current.busy) {
        runs.current += 1
        void client.cancelTranslation(leaving).catch(() => undefined)
      }
    }
  }, [client, id])

  const signature = settings
    ? JSON.stringify([configured, settings.provider, settings.targetLanguage, settings.suggest, settings.neverSuggest,
      settings.alwaysTranslate, settings.includeQuotes, settings.consentedProvider])
    : null
  useEffect(() => {
    if (!id || !eligible || signature === null) return
    let live = true
    const basis = latest.current.settings ? basisOf(latest.current.settings) : ''
    const startedAt = epoch.current
    client.translationStatus(id).then((incoming) => {
      // A translation that began or ended since this was asked has a newer answer than this one.
      if (!live || idRef.current !== id || epoch.current !== startedAt) return
      const remembered = memory.get(id)
      patch(id, (current) => {
        let status = incoming
        let override = current.override
        if (override && override.basis !== basis) override = null
        // The status call reports the configured target; a language picked from the menu stays until the setting changes.
        if (override && current.status && incoming.targetLanguage !== override.target) {
          status = { ...incoming, targetLanguage: current.status.targetLanguage, translation: current.status.translation }
        }
        let showing = current.showing
        if (!status.translation) showing = 'original'
        else if (!current.status && (remembered?.mode === 'translation' || (status.autoTranslate && remembered?.mode !== 'original'))) showing = 'translation'
        return { status, override, showing, viewed: current.viewed || showing === 'translation' }
      })
      const current = stateRef.current
      if (incoming.autoTranslate && !incoming.translation && remembered?.mode !== 'original' && !current.busy && !current.error && !current.override) {
        translate()
      }
    }, () => undefined)
    return () => { live = false }
  }, [client, id, eligible, signature, memory, patch, translate])

  const status = state.status
  const translation = status?.translation ?? null
  const showingTranslation = state.showing === 'translation' && translation !== null
  const phase: TranslationPhase = state.busy ? 'translating' : state.error ? 'error' : translation ? 'translated' : 'idle'

  const showOriginal = useCallback(() => {
    const forId = idRef.current
    if (!forId) return
    remember(forId, { mode: 'original' })
    patch(forId, { showing: 'original', viewed: true })
  }, [patch, remember])

  const showTranslation = useCallback(() => {
    const forId = idRef.current
    if (!forId) return
    remember(forId, { mode: 'translation' })
    patch(forId, { showing: 'translation', viewed: true })
  }, [patch, remember])

  const toggle = useCallback(() => {
    const current = stateRef.current
    if (current.busy) return
    if (current.id === idRef.current && current.status?.translation) {
      if (current.showing === 'translation') showOriginal()
      else showTranslation()
    } else translate()
  }, [showOriginal, showTranslation, translate])

  const dismiss = useCallback(() => {
    const forId = idRef.current
    if (!forId) return
    remember(forId, { dismissed: true })
    patch(forId, { dismissed: true })
  }, [patch, remember])

  const dismissError = useCallback(() => {
    const forId = idRef.current
    if (forId) patch(forId, { error: null })
  }, [patch])

  /** The translated document could not be prepared: say so, and go back to the original. */
  const failBody = useCallback((failure: unknown) => {
    const forId = idRef.current
    if (!forId) return
    remember(forId, { mode: 'original' })
    patch(forId, { showing: 'original', error: { failure } })
  }, [patch, remember])

  const retranslate = useCallback(() => {
    const forId = idRef.current
    if (!forId) return
    const target = stateRef.current.override?.target ?? stateRef.current.status?.targetLanguage
    void client.clearTranslation(forId).catch(() => undefined).then(() => {
      patch(forId, (current) => ({ status: current.status ? { ...current.status, translation: null } : null, showing: 'original' }))
      translate(target)
    })
  }, [client, patch, translate])

  const confirmConsent = useCallback(async () => {
    const forId = idRef.current
    const wanted = stateRef.current.consent
    const current = latest.current.settings
    if (!forId || !wanted || !current?.provider) return
    // The dialog closes at once; a second click cannot save twice.
    patch(forId, { consent: null })
    try {
      await update.mutateAsync({ consentedProvider: current.provider })
    } catch (failure) {
      patch(forId, { error: { failure } })
      return
    }
    begin(forId, wanted.target)
  }, [begin, patch, update])

  const cancelConsent = useCallback(() => {
    const forId = idRef.current
    if (forId) patch(forId, { consent: null })
  }, [patch])

  /** Adds the detected language to one list (and takes it off the other), or removes it again. */
  const toggleLanguageList = useCallback((list: 'always' | 'never') => {
    const code = stateRef.current.status?.detectedLanguage
    const current = latest.current.settings
    if (!code || !current) return
    const always = new Set(current.alwaysTranslate)
    const never = new Set(current.neverSuggest)
    const [mine, other] = list === 'always' ? [always, never] : [never, always]
    if (mine.has(code)) mine.delete(code)
    else { mine.add(code); other.delete(code) }
    update.mutate({ alwaysTranslate: [...always], neverSuggest: [...never] })
  }, [update])

  const detected = status?.detectedLanguage ?? null
  return {
    /** Whether this message can be translated at all; otherwise the feature is absent. */
    available: eligible,
    /** A service is chosen and ready: a keyless one, or one with a saved key. */
    configured,
    keyless,
    serviceName: provider ? TRANSLATION_PROVIDER_NAMES[provider] : null,
    phase,
    showing: state.showing,
    status,
    error: state.error?.failure ?? null,
    hasError: state.error !== null,
    dismissed: state.dismissed,
    /** The translation has been on screen, so "showing the original" is the user's choice. */
    viewed: state.viewed,
    target: state.override?.target ?? status?.targetLanguage ?? null,
    generation: state.generation,
    subject: showingTranslation ? translation.subject : null,
    translationVisible: showingTranslation,
    detectedLanguage: detected,
    alwaysTranslating: detected !== null && Boolean(settings?.alwaysTranslate.includes(detected)),
    neverSuggesting: detected !== null && Boolean(settings?.neverSuggest.includes(detected)),
    consentService: state.consent && provider ? TRANSLATION_PROVIDER_NAMES[provider] : null,
    translate, cancel, toggle, showOriginal, showTranslation, dismiss, dismissError, failBody, retranslate,
    confirmConsent, cancelConsent, toggleLanguageList, openSettings,
  }
}

export type EmailTranslation = ReturnType<typeof useEmailTranslation>
