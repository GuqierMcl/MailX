import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { ArrowDown01Icon, ArrowLeft01Icon, ChatAdd01Icon } from '@hugeicons/core-free-icons'
import { t } from '@/i18n'
import { Button } from '@/components/ui/button'
import { AiMark } from '@/features/ai/ai-mark'
import { AiConsentDialog } from '@/features/ai/ai-consent-dialog'
import { ChatEnvContext, type ChatEnv } from '@/features/ai/chat-env'
import { ChatHistory } from '@/features/ai/chat-history'
import { ChatInput } from '@/features/ai/chat-input'
import { ChatMessages } from '@/features/ai/chat-messages'
import type { AssistantChat } from '@/features/ai/use-assistant-chat'
import { useStickToBottom } from '@/features/ai/use-stick-to-bottom'
import { escapeBelongsToSomethingElse } from '@/features/mail/use-close-on-escape'
import { usePrefersReducedMotion } from '@/hooks/use-reduced-motion'
import type { Account, Folder } from '@/generated/accounts'
import type { Mailbox } from '@/features/mail/mailbox'
import type { MailxClient } from '@/lib/mailx-client'
import { cn } from '@/lib/utils'
import { useViewSwap } from './use-view-swap'

type View = 'overview' | 'conversation'

const WAITING_CARD = '[data-slot="confirm-card"][data-status="waiting"]'

/** A small floating button above the input: it fades in with a slight rise. */
function Pill({ onClick, children, className }: { onClick: () => void; children: ReactNode; className?: string }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn('animate-chat-rise pointer-events-auto flex max-w-full items-center gap-1.5 rounded-full bg-background px-3 py-1.5 text-xs text-foreground shadow-sm ring-1 ring-foreground/10 outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring', className)}
    >
      {children}
    </button>
  )
}

/**
 * The assistant's part of the Now page (now-page.md 6, ai-chat-ui.md 5): the input at the bottom of the
 * page and the conversation view that replaces the overview after the first message. `children` is
 * the overview. The conversation itself lives in `chat`, above the page, so it survives leaving it;
 * which of the two views shows is this page's own state (it opens on the overview).
 *
 * The overview and the conversation trade places with a short fade and a slight shift of the live
 * views (`useViewSwap`); reduced motion swaps at once. The input is mounted once outside both, so its
 * text and focus ride through the change.
 */
export function NowChatShell({ client, chat, usable, service, summary, onOpenEmail, onOpenDraft, accounts, folders, onSelectMailbox, focusToken = 0, children }: {
  client: MailxClient
  /** Null where no conversation is kept (no provider): the page is then just the overview. */
  chat: AssistantChat | null
  usable: boolean
  /** The connection's name, for the consent dialog. */
  service: string
  /** The one-line summary the conversation view keeps at its top (greeting and counts). */
  summary: string
  onOpenEmail: (email: { id: string; accountId: string }) => void
  onOpenDraft: (draftId: string) => void
  /** What the folder cards in replies are drawn from, and how a click on one opens its mailbox. */
  accounts: Account[]
  folders: Folder[]
  onSelectMailbox: (mailbox: Mailbox) => void
  /** Changes when something outside asks to start typing (the command palette). */
  focusToken?: number
  children: ReactNode
}) {
  useTranslation()
  const reduced = usePrefersReducedMotion()
    const [view, setView] = useState<View>('overview')
  const [input, setInput] = useState<HTMLTextAreaElement | null>(null)
  const footer = useRef<HTMLDivElement | null>(null)
  const shell = useRef<HTMLDivElement | null>(null)
  const [waiting, setWaiting] = useState<string | null>(null)
  const granted = useRef(false)
  const [restore, setRestore] = useState<{ text: string; key: number } | null>(null)
  const [cardShown, setCardShown] = useState(false)

  const messages = chat?.messages ?? []
  const hasMessages = messages.length > 0
  const chatOn = chat !== null && usable
  // With the assistant gone, a conversation that exists stays readable; there is just no input.
  const conversation = view === 'conversation' && chat !== null && (chatOn || hasMessages)
  const busy = chat?.busy ?? false

  const phase = useViewSwap(conversation, reduced)
  // The conversation stays on screen while it leaves. The overview behind it is out of reach (inert, hidden from
  // assistive tech) from the start of the swap, and not drawn at all once the swap is over.
  const showConversation = conversation || phase === 'to-overview'
  const overviewHidden = conversation && phase === null
  const go = useCallback((next: View) => setView(next), [])

  const { attach, node: scroller, atBottom, scrollToBottom, onScroll, onWheel } = useStickToBottom(messages, { reduced, enabled: conversation })

  // Another conversation took the screen (picked from history): start at its end. Again a frame later, once the
  // long-ago messages that the browser skips have settled to their real height.
  const viewKey = chat?.viewKey ?? 0
  useLayoutEffect(() => {
    if (viewKey === 0) return
    scrollToBottom(false)
    const frame = requestAnimationFrame(() => scrollToBottom(false))
    return () => cancelAnimationFrame(frame)
  }, [scrollToBottom, viewKey])

  const send = useCallback((message: string) => {
    if (!chat) return
    void chat.send(message).then((result) => {
      if (result === 'consent') { granted.current = false; setWaiting(message) }
      else if (result === 'sent' || result === 'failed') go('conversation')
    })
  }, [chat, go])

  const leave = useCallback(() => go('overview'), [go])
  const newChat = () => { chat?.reset(); go('overview') }
  const focusInput = useCallback(() => { input?.focus({ preventScroll: true }) }, [input])
  const scrollToCard = useCallback(() => {
    // After the conversation is on screen the card exists; look next frame.
    requestAnimationFrame(() => document.querySelector(WAITING_CARD)?.scrollIntoView({ block: 'center', behavior: reduced ? 'auto' : 'smooth' }))
  }, [reduced])

  // Esc outside the field: stops a running reply, else goes back to the overview.
  useEffect(() => {
    if (!conversation || !chat) return
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || escapeBelongsToSomethingElse(event)) return
      if (chat.busy) chat.stop()
      else leave()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [chat, conversation, leave])

  // The palette asks to type: a beat after it closes, so its own focus restoring does not win.
  const handled = useRef(0)
  useEffect(() => {
    if (!input || focusToken === handled.current) return
    const timer = setTimeout(() => { handled.current = focusToken; focusInput() }, 60)
    return () => clearTimeout(timer)
  }, [focusInput, focusToken, input])

  // The input's height decides how far the content must stay clear of it.
  useEffect(() => {
    const node = footer.current
    const root = shell.current
    if (!node || !root) return
    const measure = () => root.style.setProperty('--chat-footer-h', `${node.offsetHeight}px`)
    measure()
    if (typeof ResizeObserver === 'undefined') return () => root.style.removeProperty('--chat-footer-h')
    const observer = new ResizeObserver(measure)
    observer.observe(node)
    return () => { observer.disconnect(); root.style.removeProperty('--chat-footer-h') }
  }, [chatOn])

  // Is the card waiting for an answer in view? If not, a pill leads to it.
  const pendingId = chat?.pending?.requestId ?? null
  useEffect(() => {
    const card = pendingId && conversation ? document.querySelector(WAITING_CARD) : null
    const root = scroller()
    // Without a way to look, or a card, count it as not in view: the pill is then the way to it.
    if (!card || !root || typeof IntersectionObserver === 'undefined') return
    const observer = new IntersectionObserver(([entry]) => setCardShown(entry.isIntersecting), { root, threshold: 0.5 })
    observer.observe(card)
    return () => { observer.disconnect(); setCardShown(false) }
  }, [conversation, pendingId, scroller])

  const env = useMemo<ChatEnv | null>(() => chat && {
    client, onOpenEmail, onOpenDraft, accounts, folders, onOpenFolder: (folder) => onSelectMailbox({ kind: 'folder', folder }), respond: (approved) => { void chat.respond(approved) }, retry: () => { void chat.retry() },
    focusInput, undone: chat.undone, markUndone: chat.markUndone,
  }, [accounts, chat, client, focusInput, folders, onOpenDraft, onOpenEmail, onSelectMailbox])

  const showConfirmPill = chat?.pending && (!conversation || !cardShown)

  return (
    <div ref={shell} data-slot="now-chat" className="absolute inset-0">
      {/* The overview stays mounted behind the conversation, so its scroll position and entrance are not replayed on return. */}
      <div
        data-slot="now-overview"
        data-phase={phase ?? undefined}
        inert={conversation}
        aria-hidden={conversation || undefined}
        className={cn('absolute inset-0', overviewHidden && 'invisible', phase && 'pointer-events-none will-change-[transform,opacity]', phase === 'to-conversation' && 'animate-view-out-up', phase === 'to-overview' && 'animate-view-in-down')}
      >
        {children}
      </div>
      {showConversation && chat && env && (
        <div
          data-slot="now-conversation"
          data-phase={phase ?? undefined}
          inert={phase === 'to-overview'}
          aria-hidden={phase === 'to-overview' || undefined}
          className={cn('absolute inset-0 flex flex-col', phase && 'pointer-events-none will-change-[transform,opacity]', phase === 'to-conversation' && 'animate-view-in-up', phase === 'to-overview' && 'animate-view-out-down')}
        >
          <div className="mx-auto flex w-full max-w-190 shrink-0 items-center gap-1 px-2 pt-3 sm:px-8">
            <Button type="button" variant="ghost" size="lg" onClick={leave}>
              <HugeiconsIcon icon={ArrowLeft01Icon} aria-hidden strokeWidth={2} data-icon="inline-start" />
              {t('ai:chat.back')}
            </Button>
            <p className="min-w-0 flex-1 truncate px-2 text-sm text-muted-foreground">{summary}</p>
            <Button type="button" variant="ghost" size="lg" onClick={newChat}>
              <HugeiconsIcon icon={ChatAdd01Icon} aria-hidden strokeWidth={2} data-icon="inline-start" />
              {t('ai:chat.newConversation')}
            </Button>
            <ChatHistory client={client} chat={chat} onStartOver={newChat} />
          </div>
          <div ref={attach} onScroll={onScroll} onWheel={onWheel} data-slot="chat-scroller" className="relative min-h-0 flex-1 overflow-y-auto [overflow-anchor:none]">
            <div className="mx-auto w-full max-w-190 px-5 pt-4 sm:px-10" style={{ paddingBottom: 'calc(var(--chat-footer-h, 6rem) + 1.5rem)' }}>
              {/* A different conversation replaces the list under a new key: it fades in and none of its messages replay their entrance. */}
              <div key={viewKey} className={cn(viewKey > 0 && !reduced && 'animate-now-text-in')}>
                <ChatEnvContext value={env}>
                  <ChatMessages messages={messages} />
                </ChatEnvContext>
              </div>
            </div>
          </div>
        </div>
      )}

      {chatOn && chat && (
        <div
          ref={footer}
          data-slot="chat-footer"
          className={cn('pointer-events-none absolute inset-x-0 bottom-0 z-10 px-5 pt-8 pb-4 sm:px-10', !reduced && 'animate-chat-input-in')}
        >
          <div aria-hidden className="absolute inset-0 bg-linear-to-t from-background from-45% via-background/85 to-transparent" />
          <div className="relative mx-auto flex max-w-190 flex-col">
            <div className="mb-2 flex flex-wrap items-end justify-center gap-2 empty:hidden">
              {showConfirmPill && (
                <Pill onClick={() => { if (conversation) scrollToCard(); else { go('conversation'); scrollToCard() } }}>
                  <span className="size-1.5 rounded-full bg-amber-500" aria-hidden />
                  {t('ai:chat.confirmWaiting')}
                </Pill>
              )}
              {!conversation && hasMessages && (
                <Pill onClick={() => go('conversation')}>
                  <AiMark />
                  {busy ? t('ai:chat.resumeWorking') : t('ai:chat.resume')}
                </Pill>
              )}
              {conversation && !atBottom && (
                <Pill onClick={() => scrollToBottom()}>
                  <HugeiconsIcon icon={ArrowDown01Icon} aria-hidden strokeWidth={2} className="size-3.5" />
                  {t('ai:chat.backToBottom')}
                </Pill>
              )}
            </div>
            <div className="pointer-events-auto">
              <ChatInput working={busy} onSend={send} onStop={chat.stop} onEscape={leave} inputRef={setInput} restore={restore} />
            </div>
            {!chat.toolsAvailable && <p className="pointer-events-auto px-4 pt-2 text-xs text-muted-foreground">{t('ai:chat.noTools')}</p>}
          </div>
        </div>
      )}
      <AiConsentDialog
        client={client}
        service={service}
        open={waiting !== null}
        onOpenChange={(open) => {
          if (open) return
          const message = waiting
          setWaiting(null)
          // Declined: the words go back into the field. Agreeing sets `granted` right after closing, in the same turn.
          queueMicrotask(() => { if (message && !granted.current) setRestore({ text: message, key: Date.now() }) })
        }}
        onGranted={() => { granted.current = true; if (waiting) send(waiting) }}
      />
    </div>
  )
}
