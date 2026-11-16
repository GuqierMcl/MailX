import { memo, useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import { Button } from '@/components/ui/button'
import { AiGeneratedMark } from './ai-mark'
import { confirmStatus, turnFragments, turnWorking, type ChatMessage, type ChatTurn } from './chat-model'
import { useChatEnv } from './chat-env'
import { ConfirmCard, ConfirmOutcomeLine } from './confirm-card'
import { Markdown } from './markdown-view'
import { ToolRow } from './tool-parts'
import { cn } from '@/lib/utils'

/** After this long without a first word, the thinking line says the model is slow to answer. */
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

function UserMessage({ text, fresh }: { text: string; fresh: boolean }) {
  return (
    <div data-slot="chat-user" className={cn('max-w-[80%] self-end rounded-2xl bg-muted px-3.5 py-2 text-[0.9375rem] leading-relaxed wrap-anywhere whitespace-pre-wrap', fresh && 'animate-chat-rise')}>
      {text}
    </div>
  )
}

/** One reply, part by part. Only the part still being written changes while it streams. */
const AssistantMessage = memo(function AssistantMessage({ turn, latest }: { turn: ChatTurn; latest: boolean }) {
  useTranslation()
  const env = useChatEnv()
  const fragments = turnFragments(turn)
  const working = turnWorking(turn)
  const thinking = working && turn.state !== 'awaitingConfirmation' && fragments.length === 0
  const slow = useAfter(thinking, SLOW_REPLY_MS)
  const openLink = (url: string) => { void env.client.openExternalLink(url).catch(() => undefined) }
  const failed = turn.state === 'failed'
  return (
    <div data-slot="chat-assistant" data-state={turn.state} className={cn('animate-now-text-in relative flex flex-col gap-2.5 pl-6')}>
      <span className="absolute top-[7px] left-0"><AiGeneratedMark at={turn.finishedAt} /></span>
      {fragments.map((fragment) => {
        switch (fragment.kind) {
          case 'text': return <Markdown key={fragment.key} text={fragment.text} streaming={fragment.streaming} onOpenLink={openLink} />
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
              ? <UserMessage text={message.text} fresh={index >= initial} />
              : <AssistantMessage turn={message.turn} latest={last} />}
          </div>
        )
      })}
    </div>
  )
}
