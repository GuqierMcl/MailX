import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { ArrowDown01Icon, Cancel01Icon, ChatAdd01Icon, CheckListIcon, Mail01Icon, MailEdit01Icon } from '@hugeicons/core-free-icons'
import { t } from '@/i18n'
import { IconButton } from '@/components/icon-button'
import { Island } from '@/features/shell/island'
import { ASSISTANT_DEFAULT, ASSISTANT_MAX, ASSISTANT_MIN, clampAssistantWidth } from '@/features/shell/layout-defaults'
import { usePrefersReducedMotion } from '@/hooks/use-reduced-motion'
import type { Account, ChatContext, Folder } from '@/generated/accounts'
import type { Mailbox } from '@/features/mail/mailbox'
import type { MailxClient } from '@/lib/mailx-client'
import { cn } from '@/lib/utils'
import { AiConsentDialog } from './ai-consent-dialog'
import { AiMark } from './ai-mark'
import { ChatEnvContext, type ChatEnv } from './chat-env'
import { ChatHistory } from './chat-history'
import { ChatInput } from './chat-input'
import { ChatMessages } from './chat-messages'
import { useSharedAssistantChat } from './assistant-chat-context'
import { useStickToBottom } from './use-stick-to-bottom'

/** What the user is looking at, as the island offers it to the next message. */
export type AssistantView = {
  context: ChatContext
  /** A short name for it: a subject, or "3 selected emails". */
  label: string
}

/** Same view, same key: a dismissed view stays dismissed until the screen shows something else. */
const viewKey = (view: AssistantView | null) => (view ? JSON.stringify(view.context) : '')

const VIEW_ICON = { email: Mail01Icon, emails: CheckListIcon, draft: MailEdit01Icon } as const

function placeholder(view: AssistantView | null) {
  switch (view?.context.type) {
    case 'email': return t('ai:island.placeholderEmail')
    case 'emails': return t('ai:island.placeholderEmails')
    case 'draft': return t('ai:island.placeholderDraft')
    default: return t('ai:chat.placeholder')
  }
}

const WAITING_CARD = '[data-slot="confirm-card"][data-status="waiting"]'

function Pill({ onClick, children }: { onClick: () => void; children: ReactNode }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="animate-chat-rise pointer-events-auto flex max-w-full items-center gap-1.5 rounded-full bg-background px-3 py-1.5 text-xs text-foreground shadow-sm ring-1 ring-foreground/10 outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring"
    >
      {children}
    </button>
  )
}

/**
 * The global AI capsule (ai-chat-ui.md 5.1): a small field-like button at the bottom right of the mail
 * island. A click opens the assistant's own island; the label says what it would be asked about.
 */
export function AssistantCapsule({ view, raised, onOpen }: {
  view: AssistantView | null
  /** Lifted clear of a composer's bottom bar. */
  raised: boolean
  onOpen: () => void
}) {
  useTranslation()
  return (
    <button
      type="button"
      data-slot="assistant-capsule"
      onClick={onOpen}
      aria-keyshortcuts="Meta+J Control+J"
      className={cn(
        'animate-chat-rise group/capsule absolute right-4 z-20 flex h-9 w-60 max-w-[calc(100%-2rem)] items-center gap-2 rounded-full bg-background/95 pr-3 pl-3.5 text-left text-sm text-muted-foreground shadow-md ring-1 ring-foreground/10 outline-none backdrop-blur-sm transition-[bottom,box-shadow] duration-(--motion-medium) ease-(--ease-out-soft) hover:text-foreground hover:shadow-lg focus-visible:ring-2 focus-visible:ring-ring dark:bg-card/95',
        raised ? 'bottom-16' : 'bottom-4',
      )}
    >
      <AiMark className="size-4" />
      <span className="min-w-0 flex-1 truncate">{placeholder(view)}</span>
    </button>
  )
}

/** The left edge of the island: drag (or arrow keys) to change its width; a double click resets it. */
function WidthHandle({ width, onResize, onCommit }: {
  width: number
  onResize: (width: number) => void
  onCommit: (width: number) => void
}) {
  useTranslation()
  const drag = useRef<{ x: number; width: number; last: number } | null>(null)
  const down = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return
    event.preventDefault()
    event.currentTarget.setPointerCapture(event.pointerId)
    drag.current = { x: event.clientX, width, last: width }
  }
  const move = (event: PointerEvent<HTMLDivElement>) => {
    const start = drag.current
    if (!start) return
    // The island grows to the left: moving the pointer left widens it.
    start.last = clampAssistantWidth(start.width + start.x - event.clientX)
    onResize(start.last)
  }
  const up = () => {
    const start = drag.current
    drag.current = null
    if (start && start.last !== start.width) onCommit(start.last)
  }
  const key = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = event.shiftKey ? 64 : 16
    const next = event.key === 'ArrowLeft' ? width + step : event.key === 'ArrowRight' ? width - step
      : event.key === 'Home' ? ASSISTANT_MAX : event.key === 'End' ? ASSISTANT_MIN : null
    if (next === null) return
    event.preventDefault()
    const clamped = clampAssistantWidth(next)
    onResize(clamped)
    onCommit(clamped)
  }
  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={t('ai:island.resize')}
      aria-valuemin={ASSISTANT_MIN}
      aria-valuemax={ASSISTANT_MAX}
      aria-valuenow={width}
      tabIndex={0}
      onPointerDown={down}
      onPointerMove={move}
      onPointerUp={up}
      onPointerCancel={up}
      onDoubleClick={() => { onResize(ASSISTANT_DEFAULT); onCommit(ASSISTANT_DEFAULT) }}
      onKeyDown={key}
      className="group/handle absolute inset-y-4 -left-2 z-30 flex w-2 cursor-col-resize touch-none justify-center outline-none"
    >
      <span aria-hidden className="h-full w-0.5 rounded-full bg-transparent transition-colors duration-(--motion-fast) group-hover/handle:bg-foreground/15 group-focus-visible/handle:bg-ring group-active/handle:bg-foreground/25" />
    </div>
  )
}

/**
 * The assistant's own island beside the mail (ai-chat-ui.md 5.1). It shows the shared conversation (the
 * same one as the Now page) and offers what is on screen as the context of the next message: a chip
 * above the input that can be dismissed for as long as the view stays the same. Esc inside stops a reply
 * and never closes the mail beside it; the island closes with its button or Cmd/Ctrl+J.
 *
 * `overlay` floats it over the mail island when the window is too narrow for both side by side.
 */
export function AssistantIsland(props: Omit<IslandProps, 'chat'>) {
  const chat = useSharedAssistantChat()
  return chat ? <AssistantIslandView {...props} chat={chat} /> : null
}

type IslandProps = {
  client: MailxClient
  chat: NonNullable<ReturnType<typeof useSharedAssistantChat>>
  service: string
  view: AssistantView | null
  /** The saved width; dragging shows the new one at once and saves it when the drag ends. */
  width: number
  overlay: boolean
  onWidthCommit: (width: number) => void
  onClose: () => void
  onOpenEmail: (email: { id: string; accountId: string }) => void
  onOpenDraft: (draftId: string) => void
  accounts: Account[]
  folders: Folder[]
  onSelectMailbox: (mailbox: Mailbox) => void
  /** Changes when something outside asks to start typing. */
  focusToken: number
}

function AssistantIslandView({ client, chat, service, view, width, overlay, onWidthCommit, onClose, onOpenEmail, onOpenDraft, accounts, folders, onSelectMailbox, focusToken }: IslandProps) {
  useTranslation()
  const reduced = usePrefersReducedMotion()
  const [input, setInput] = useState<HTMLTextAreaElement | null>(null)
  const [waiting, setWaiting] = useState<{ message: string; context: ChatContext | null } | null>(null)
  const granted = useRef(false)
  const [restore, setRestore] = useState<{ text: string; key: number } | null>(null)
  const [dismissed, setDismissed] = useState<string | null>(null)
  const [cardShown, setCardShown] = useState(false)
  const [dragWidth, setDragWidth] = useState<{ from: number; value: number } | null>(null)
  const shown = dragWidth && dragWidth.from === width ? dragWidth.value : width

  const key = viewKey(view)
  const activeView = view && key !== dismissed ? view : null
  const messages = chat.messages
  const { attach, node: scroller, atBottom, scrollToBottom, onScroll, onWheel } = useStickToBottom(messages, { reduced })

  const focusInput = useCallback(() => { input?.focus({ preventScroll: true }) }, [input])
  // Opening the island (and every later request to type) puts the caret in the field.
  const handled = useRef(-1)
  useEffect(() => {
    if (!input || focusToken === handled.current) return
    handled.current = focusToken
    const frame = requestAnimationFrame(focusInput)
    return () => cancelAnimationFrame(frame)
  }, [focusInput, focusToken, input])

  const viewKeyOfChat = chat.viewKey
  useLayoutEffect(() => {
    if (viewKeyOfChat === 0) return
    scrollToBottom(false)
    const frame = requestAnimationFrame(() => scrollToBottom(false))
    return () => cancelAnimationFrame(frame)
  }, [scrollToBottom, viewKeyOfChat])

  const send = useCallback((message: string, context: ChatContext | null) => {
    void chat.send(message, context).then((result) => {
      if (result === 'consent') { granted.current = false; setWaiting({ message, context }) }
    })
  }, [chat])

  const newChat = useCallback(() => { chat.reset(); focusInput() }, [chat, focusInput])

  const pendingId = chat.pending?.requestId ?? null
  useEffect(() => {
    const card = pendingId ? document.querySelector(WAITING_CARD) : null
    const root = scroller()
    if (!card || !root || typeof IntersectionObserver === 'undefined') return
    const observer = new IntersectionObserver(([entry]) => setCardShown(entry.isIntersecting), { root, threshold: 0.5 })
    observer.observe(card)
    return () => { observer.disconnect(); setCardShown(false) }
  }, [pendingId, scroller])

  const env = useMemo<ChatEnv>(() => ({
    client, onOpenEmail, onOpenDraft, accounts, folders, onOpenFolder: (folder) => onSelectMailbox({ kind: 'folder', folder }),
    respond: (approved) => { void chat.respond(approved) }, retry: () => { void chat.retry() },
    focusInput, undone: chat.undone, markUndone: chat.markUndone,
  }), [accounts, chat, client, focusInput, folders, onOpenDraft, onOpenEmail, onSelectMailbox])

  // Esc anywhere in the island stops a reply; the mail behind it stays as it is (the workbench leaves Esc from here alone).
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape' && !event.defaultPrevented && chat.busy) chat.stop()
  }

  return (
    <Island
      data-slot="assistant-island"
      data-overlay={overlay || undefined}
      aria-label={t('ai:island.title')}
      role="complementary"
      onKeyDown={onKeyDown}
      style={{ width: shown }}
      className={cn(
        'flex-none overflow-visible',
        overlay ? 'absolute top-0 right-2 bottom-2 z-40 shadow-xl' : 'ml-2',
        !reduced && 'animate-in fade-in-0 slide-in-from-right-4 duration-(--motion-view) ease-(--ease-out-soft)',
      )}
    >
      {!overlay && (
        <WidthHandle
          width={shown}
          onResize={(value) => setDragWidth({ from: width, value })}
          onCommit={onWidthCommit}
        />
      )}
      <div className="flex min-h-0 flex-1 flex-col overflow-hidden rounded-[inherit]">
        <div className="flex shrink-0 items-center gap-1 py-2 pr-2 pl-4">
          <AiMark className="size-4" />
          <h2 className="min-w-0 flex-1 truncate pl-1 text-sm font-medium">{t('ai:island.title')}</h2>
          <IconButton label={t('ai:chat.newConversation')} size="icon-lg" onClick={newChat}>
            <HugeiconsIcon icon={ChatAdd01Icon} aria-hidden strokeWidth={2} />
          </IconButton>
          <ChatHistory client={client} chat={chat} onStartOver={newChat} />
          <IconButton label={t('ai:island.close')} size="icon-lg" onClick={onClose}>
            <HugeiconsIcon icon={Cancel01Icon} aria-hidden strokeWidth={2} />
          </IconButton>
        </div>
        <div ref={attach} onScroll={onScroll} onWheel={onWheel} data-slot="chat-scroller" className="relative min-h-0 flex-1 overflow-y-auto [overflow-anchor:none]">
          {messages.length === 0 ? (
            <div className="flex h-full flex-col items-center justify-center gap-2 px-8 text-center text-sm text-muted-foreground">
              <AiMark className="size-6" />
              <p>{t('ai:island.empty')}</p>
            </div>
          ) : (
            <div className="px-4 pt-2 pb-4">
              <div key={viewKeyOfChat} className={cn(viewKeyOfChat > 0 && !reduced && 'animate-now-text-in')}>
                <ChatEnvContext value={env}>
                  <ChatMessages messages={messages} />
                </ChatEnvContext>
              </div>
            </div>
          )}
        </div>
        <div className="relative shrink-0 px-3 pb-3">
          <div className="pointer-events-none absolute inset-x-0 -top-10 flex flex-wrap items-end justify-center gap-2 px-3 empty:hidden">
            {chat.pending && !cardShown && (
              <Pill onClick={() => document.querySelector(WAITING_CARD)?.scrollIntoView({ block: 'center', behavior: reduced ? 'auto' : 'smooth' })}>
                <span className="size-1.5 rounded-full bg-amber-500" aria-hidden />
                {t('ai:chat.confirmWaiting')}
              </Pill>
            )}
            {!chat.pending && !atBottom && messages.length > 0 && (
              <Pill onClick={() => scrollToBottom()}>
                <HugeiconsIcon icon={ArrowDown01Icon} aria-hidden strokeWidth={2} className="size-3.5" />
                {t('ai:chat.backToBottom')}
              </Pill>
            )}
          </div>
          {activeView && (
            <div data-slot="assistant-view" className="mb-2 flex min-w-0 items-center gap-1.5 rounded-full bg-muted/60 py-1 pr-1 pl-2.5 text-xs text-muted-foreground">
              <HugeiconsIcon icon={VIEW_ICON[activeView.context.type]} aria-hidden strokeWidth={2} className="size-3.5 shrink-0" />
              <span className="min-w-0 flex-1 truncate" title={activeView.label}>{activeView.label}</span>
              <IconButton label={t('ai:island.dropView')} size="icon-xs" className="rounded-full" onClick={() => setDismissed(key)}>
                <HugeiconsIcon icon={Cancel01Icon} aria-hidden strokeWidth={2} />
              </IconButton>
            </div>
          )}
          <ChatInput
            working={chat.busy}
            onSend={(message) => send(message, activeView?.context ?? null)}
            onStop={chat.stop}
            onEscape={() => undefined}
            inputRef={setInput}
            placeholder={placeholder(activeView)}
            restore={restore}
          />
          {!chat.toolsAvailable && <p className="px-3 pt-2 text-xs text-muted-foreground">{t('ai:chat.noTools')}</p>}
        </div>
      </div>
      <AiConsentDialog
        client={client}
        service={service}
        open={waiting !== null}
        onOpenChange={(open) => {
          if (open) return
          const asked = waiting
          setWaiting(null)
          queueMicrotask(() => { if (asked && !granted.current) setRestore({ text: asked.message, key: Date.now() }) })
        }}
        onGranted={() => { granted.current = true; if (waiting) send(waiting.message, waiting.context) }}
      />
    </Island>
  )
}
