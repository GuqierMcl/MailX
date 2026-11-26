import { memo, useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { ArrowRight01Icon, Copy01Icon, PencilEdit02Icon, RefreshIcon } from '@hugeicons/core-free-icons'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import { Button } from '@/components/ui/button'
import { toast } from '@/components/ui/toast'
import { AiGeneratedMark } from './ai-mark'
import {
  confirmStatus, messageTime, turnCanContinue, turnChangedMail, turnFragments, turnWorking,
  type ChatMessage, type ChatTurn,
} from './chat-model'
import { useChatEnv } from './chat-env'
import { ConfirmCard, ConfirmOutcomeLine } from './confirm-card'
import { Markdown } from './markdown-view'
import { ThinkingRow } from './thinking-row'
import { ToolRow } from './tool-parts'
import { cn } from '@/lib/utils'

/** After this long with nothing at all from the model, the thinking line says it is slow to answer. */
export const SLOW_REPLY_MS = 10_000
/** Messages this far from the end are left to the browser to skip while off screen. */
const RECENT_MESSAGES = 4

/** True once `active` has lasted `ms`. Stays true after: a reply that got going does not slow down again. */
function useAfter(active: boolean, ms: number) {
  const [late, setLate] = useState(false)
  useEffect(() => {
    if (!active) return
    const timer = setTimeout(() => setLate(true), ms)
    return () => clearTimeout(timer)
  }, [active, ms])
  return active && late
}

/** The time a message was said, in the reader's clock; the full date on hover. */
function MessageTime({ at }: { at: Date }) {
  const { i18n } = useTranslation()
  const clock = new Intl.DateTimeFormat(i18n.language, { hour: '2-digit', minute: '2-digit' })
  const full = new Intl.DateTimeFormat(i18n.language, { dateStyle: 'medium', timeStyle: 'short' })
  return <time dateTime={at.toISOString()} title={full.format(at)} className="text-xs text-muted-foreground tabular-nums">{clock.format(at)}</time>
}

/** A small action under a message, shown when the message is hovered or focused. */
function MessageAction({ label, icon, onClick, disabled }: { label: string; icon: ReactNode; onClick: () => void; disabled?: boolean }) {
  return (
    <Button type="button" variant="ghost" size="icon-sm" className="size-6 text-muted-foreground" aria-label={label} title={label} disabled={disabled} onClick={onClick}>
      {icon}
    </Button>
  )
}

/** What can be done with a message, and when it was said; nothing shows until the message is hovered or focused. */
function MessageActions({ children, at, align }: { children: ReactNode; at: Date | null; align: 'start' | 'end' }) {
  useTranslation()
  return (
    <div
      role="group"
      aria-label={t('ai:chat.actions')}
      className={cn(
        'flex items-center gap-0.5 opacity-0 transition-opacity duration-150 group-hover:opacity-100 focus-within:opacity-100',
        align === 'end' ? 'self-end' : 'self-start',
      )}
    >
      {children}
      {at && <MessageTime at={at} />}
    </div>
  )
}

function copyText(text: string) {
  void navigator.clipboard.writeText(text).then(
    () => toast.success(t('ai:chat.copied')),
    () => undefined,
  )
}

/** Enter that is really part of typing (IME composition) must not send. */
const composing = (event: KeyboardEvent) => event.nativeEvent.isComposing || event.keyCode === 229

/** A message of yours being reworded in place: Enter sends it in place of the old one, Esc leaves it as it was. */
function MessageEditor({ text, onSend, onCancel }: { text: string; onSend: (text: string) => void; onCancel: () => void }) {
  useTranslation()
  const [value, setValue] = useState(text)
  const field = useRef<HTMLTextAreaElement | null>(null)
  const changed = value.trim() !== '' && value.trim() !== text.trim()
  useLayoutEffect(() => {
    const element = field.current
    if (!element) return
    element.style.height = 'auto'
    element.style.height = `${Math.min(element.scrollHeight, 240)}px`
  }, [value])
  useEffect(() => {
    const element = field.current
    element?.focus({ preventScroll: true })
    element?.setSelectionRange(element.value.length, element.value.length)
  }, [])
  const send = () => { if (changed) onSend(value.trim()) }
  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (composing(event)) return
    if (event.key === 'Enter' && !event.shiftKey && !event.ctrlKey && !event.metaKey && !event.altKey) {
      event.preventDefault()
      send()
    } else if (event.key === 'Escape') {
      event.preventDefault()
      event.stopPropagation()
      onCancel()
    }
  }
  return (
    <div data-slot="chat-user-edit" className="flex w-[80%] flex-col gap-2 self-end rounded-2xl bg-muted px-3.5 py-2">
      <textarea
        ref={field}
        aria-label={t('ai:chat.editField')}
        value={value}
        rows={1}
        onChange={(event) => setValue(event.target.value)}
        onKeyDown={onKeyDown}
        className="w-full resize-none bg-transparent text-[0.9375rem] leading-relaxed outline-none"
      />
      <div className="flex justify-end gap-1.5">
        <Button type="button" variant="ghost" size="sm" onClick={onCancel}>{t('ai:chat.editCancel')}</Button>
        <Button type="button" size="sm" disabled={!changed} onClick={send}>{t('ai:chat.send')}</Button>
      </div>
    </div>
  )
}

function UserMessage({ message, fresh, editable }: { message: Extract<ChatMessage, { role: 'user' }>; fresh: boolean; editable: boolean }) {
  useTranslation()
  const env = useChatEnv()
  const [editing, setEditing] = useState(false)
  if (editing && editable) {
    return (
      <MessageEditor
        text={message.text}
        onCancel={() => setEditing(false)}
        onSend={(text) => { setEditing(false); env.onEdit(message.id, text) }}
      />
    )
  }
  return (
    <div className="group flex flex-col items-end gap-1" data-slot="chat-user-row">
      <div data-slot="chat-user" className={cn('max-w-[80%] self-end rounded-2xl bg-muted px-3.5 py-2 text-[0.9375rem] leading-relaxed wrap-anywhere whitespace-pre-wrap', fresh && 'animate-chat-rise')}>
        {message.text}
      </div>
      <MessageActions at={messageTime(message)} align="end">
        <MessageAction label={t('ai:chat.copy')} icon={<HugeiconsIcon icon={Copy01Icon} strokeWidth={2} className="size-3.5" />} onClick={() => copyText(message.text)} />
        {editable && <MessageAction label={t('ai:chat.edit')} icon={<HugeiconsIcon icon={PencilEdit02Icon} strokeWidth={2} className="size-3.5" />} onClick={() => setEditing(true)} />}
      </MessageActions>
    </div>
  )
}

/** One reply, part by part. Only the part still being written changes while it streams. */
const AssistantMessage = memo(function AssistantMessage({ turn, latest }: { turn: ChatTurn; latest: boolean }) {
  useTranslation()
  const env = useChatEnv()
  const fragments = turnFragments(turn)
  const working = turnWorking(turn)
  // The generic line covers only the wait before anything arrives; a thinking stretch has its own line.
  const thinking = working && turn.state !== 'awaitingConfirmation' && fragments.length === 0
  const slow = useAfter(thinking, SLOW_REPLY_MS)
  const openLink = (url: string) => { void env.client.openExternalLink(url).catch(() => undefined) }
  const failed = turn.state === 'failed'
  return (
    <div data-slot="chat-assistant" data-state={turn.state} className={cn('group animate-now-text-in relative flex flex-col gap-2.5 pl-6')}>
      {/* As tall as any first row (a line of text, a thinking or tool row: 24px), so the mark sits on its middle whatever comes first. */}
      <span className="absolute top-0 left-0 flex h-6 items-center"><AiGeneratedMark at={turn.finishedAt} /></span>
      {fragments.map((fragment) => {
        switch (fragment.kind) {
          case 'text': return <Markdown key={fragment.key} text={fragment.text} streaming={fragment.streaming} onOpenLink={openLink} />
          case 'thought': return <ThinkingRow key={fragment.key} thought={fragment.thought} working={working} />
          case 'tool': return <ToolRow key={fragment.key} tool={fragment.tool} />
          case 'confirm': return (
            <ConfirmCard
              key={fragment.key}
              request={fragment.confirm.request}
              status={confirmStatus(turn, fragment.confirm.requestId)}
              onRespond={env.respond}
              onAnswered={env.focusInput}
            />
          )
          case 'outcome': return <ConfirmOutcomeLine key={fragment.key} outcome={fragment.outcome} />
        }
      })}
      {thinking && (
        <p className="text-sm text-muted-foreground">
          <span className="ai-shimmer">{t('ai:chat.thinking')}</span>
          {slow && <span className="ml-2">{t('ai:chat.thinkingSlow')}</span>}
        </p>
      )}
      {turn.state === 'canceled' && <p className="text-sm text-muted-foreground">{t('ai:chat.stopped')}</p>}
      {failed && (
        <p className="flex flex-wrap items-center gap-x-2 text-sm text-muted-foreground">
          <span>{errorText(turn.error ? { code: turn.error } : null)}</span>
          {latest && !turn.saved && <Button type="button" variant="ghost" size="lg" onClick={env.retry}>{t('ai:chat.retry')}</Button>}
        </p>
      )}
      {!working && (
        <MessageActions at={messageTime({ id: '', role: 'assistant', turn })} align="start">
          {turn.text.trim() !== '' && <MessageAction label={t('ai:chat.copy')} icon={<HugeiconsIcon icon={Copy01Icon} strokeWidth={2} className="size-3.5" />} onClick={() => copyText(turn.text)} />}
          {latest && !turnChangedMail(turn) && <MessageAction label={t('ai:chat.regenerate')} icon={<HugeiconsIcon icon={RefreshIcon} strokeWidth={2} className="size-3.5" />} onClick={env.regenerate} />}
          {latest && turnCanContinue(turn) && <MessageAction label={t('ai:chat.continue')} icon={<HugeiconsIcon icon={ArrowRight01Icon} strokeWidth={2} className="size-3.5" />} onClick={env.continueReply} />}
        </MessageActions>
      )}
      {/* Announced once, when the reply is complete, not as it is written; the text itself is read in the log. */}
      <span role="status" className="sr-only">
        {turn.state === 'completed' ? t('ai:chat.replyDone') : turn.state === 'canceled' ? t('ai:chat.replyStopped') : failed ? t('ai:chat.replyFailed') : ''}
      </span>
    </div>
  )
})

/**
 * The conversation as a log (ai-chat-ui.md 4.1, 8): the user's words on the right in a faint fill,
 * the assistant's replies as plain body text. Messages that were there when the view opened do not
 * animate; older ones are left to `content-visibility`.
 */
export function ChatMessages({ messages, className }: { messages: ChatMessage[]; className?: string }) {
  useTranslation()
  // How many messages existed when this view appeared; those never play the entrance.
  const [initial] = useState(messages.length)
  // A message of yours can be reworded while nothing after it changed mail and no reply is being written.
  const lastMessage = messages.at(-1)
  const working = lastMessage?.role === 'assistant' && turnWorking(lastMessage.turn)
  const editable = new Set<string>()
  for (let index = messages.length - 1, blocked = working; index >= 0; index -= 1) {
    const message = messages[index]
    if (message.role === 'assistant') blocked ||= turnChangedMail(message.turn)
    else if (!blocked) editable.add(message.id)
  }
  return (
    <div role="log" aria-label={t('ai:chat.list')} data-slot="chat-messages" className={cn('flex flex-col', className)}>
      {messages.map((message, index) => {
        const last = index === messages.length - 1
        const previous = messages[index - 1]
        const gap = index === 0 ? '' : previous.role === message.role ? 'mt-3' : 'mt-6'
        const old = index < messages.length - RECENT_MESSAGES
        return (
          <div key={message.id} className={cn('flex flex-col', gap, old && '[contain-intrinsic-size:auto_4rem] [content-visibility:auto]')}>
            {message.role === 'user'
              ? <UserMessage message={message} fresh={index >= initial} editable={editable.has(message.id)} />
              : <AssistantMessage turn={message.turn} latest={last} />}
          </div>
        )
      })}
    </div>
  )
}
