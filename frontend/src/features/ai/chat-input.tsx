import { useImperativeHandle, useLayoutEffect, useRef, useState, type KeyboardEvent, type Ref } from 'react'
import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { ArrowUp02Icon, StopIcon } from '@hugeicons/core-free-icons'
import { t } from '@/i18n'
import { usePrefersReducedMotion } from '@/hooks/use-reduced-motion'
import { cn } from '@/lib/utils'
import { useDocumentVisible } from './use-document-visible'

/** Taller than this and the field scrolls inside: about six lines. */
const MAX_HEIGHT = 144

/** Enter that is really part of typing (IME composition) must not send. */
const composing = (event: KeyboardEvent) => event.nativeEvent.isComposing || event.keyCode === 229

/**
 * The assistant's input (ai-chat-ui.md 4.3): a one-line capsule that grows to about six lines.
 * Enter sends, Shift+Enter breaks the line, composing text never sends. One round button is "send"
 * with something typed and "stop" while a reply is on its way; the two cross-fade in place at the same
 * size. Focus draws a 1px AI gradient border; while `working` the border drifts and a faint glow sits
 * behind it (only then; still when the window is hidden or motion is reduced).
 * The field stays focusable while a reply runs, but ignores typing: stopping with Esc must keep working.
 */
export function ChatInput({ working, onSend, onStop, onEscape, inputRef, placeholder, restore, className }: {
  working: boolean
  /** Receives the trimmed text; the field clears at once. */
  onSend: (text: string) => void
  onStop: () => void
  /** Esc with nothing to stop. */
  onEscape: () => void
  inputRef?: Ref<HTMLTextAreaElement>
  placeholder?: string
  /** Puts words back into the field (a message that could not be sent yet); a new `key` is a new request. */
  restore?: { text: string; key: number } | null
  className?: string
}) {
  useTranslation()
  const reduced = usePrefersReducedMotion()
  const visible = useDocumentVisible()
  const [text, setText] = useState('')
  const field = useRef<HTMLTextAreaElement | null>(null)
  const canSend = !working && text.trim() !== ''

  // The caller's ref points at the field itself.
  useImperativeHandle(inputRef, () => field.current as HTMLTextAreaElement, [])

  // Words handed back (a message that could not be sent yet) are taken in once per request.
  const [seen, setSeen] = useState(restore)
  if (restore !== seen) {
    setSeen(restore)
    if (restore) setText(restore.text)
  }

  // Fit the height to the text. Setting the old pixel height back before the new one lets it ease between them.
  useLayoutEffect(() => {
    const element = field.current
    if (!element) return
    const before = element.offsetHeight
    element.style.height = 'auto'
    const next = Math.min(element.scrollHeight, MAX_HEIGHT)
    if (before > 0) element.style.height = `${before}px`
    void element.offsetHeight
    element.style.height = `${next}px`
    element.style.overflowY = element.scrollHeight > MAX_HEIGHT ? 'auto' : 'hidden'
  }, [text])

  const submit = () => {
    if (!canSend) return
    const message = text.trim()
    setText('')
    onSend(message)
  }
  const onKeyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === 'Enter' && !event.shiftKey && !event.ctrlKey && !event.metaKey && !event.altKey && !composing(event)) {
      event.preventDefault()
      submit()
    } else if (event.key === 'Escape' && !composing(event)) {
      event.preventDefault()
      event.stopPropagation()
      if (working) onStop()
      else onEscape()
    }
  }
  const fade = reduced ? 'duration-100' : 'duration-150'
  return (
    <div data-slot="chat-input" data-working={working} className={cn('group/chat-input relative isolate rounded-[1.375rem]', className)}>
      {/* Behind everything: a faint, fixed blur of the gradient that only fades in while working. */}
      <div aria-hidden className={cn('ai-glow absolute -inset-0.5 -z-10 rounded-[inherit] opacity-0 transition-opacity', fade, working && 'opacity-[0.28]')} />
      <div className="relative rounded-[inherit] bg-background shadow-xs ring-1 ring-foreground/10 dark:bg-card">
        <textarea
          ref={field}
          rows={1}
          value={text}
          readOnly={working}
          aria-label={placeholder ?? t('ai:chat.placeholder')}
          placeholder={placeholder ?? t('ai:chat.placeholder')}
          onChange={(event) => { if (!working) setText(event.target.value) }}
          onKeyDown={onKeyDown}
          className={cn(
            'block max-h-36 min-h-11 w-full resize-none bg-transparent py-[11px] pr-13 pl-4 text-[0.9375rem] leading-[22px] outline-none placeholder:text-muted-foreground',
            !reduced && 'transition-[height] duration-200 ease-(--ease-out-soft)',
            working && 'text-muted-foreground',
          )}
        />
        <button
          type="button"
          aria-label={working ? t('ai:chat.stop') : t('ai:chat.send')}
          aria-disabled={!working && !canSend}
          onClick={() => (working ? onStop() : submit())}
          className={cn(
            'absolute right-1.5 bottom-1.5 flex size-8 items-center justify-center rounded-full bg-foreground text-background outline-none transition-opacity focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background',
            fade,
            !working && !canSend && 'opacity-30',
          )}
        >
          {/* Both icons stay mounted and trade places, so the button never changes size. */}
          <HugeiconsIcon icon={ArrowUp02Icon} aria-hidden strokeWidth={2.5} className={cn('absolute size-4 transition-opacity', fade, working ? 'opacity-0' : 'opacity-100')} />
          <HugeiconsIcon icon={StopIcon} aria-hidden strokeWidth={2.5} className={cn('absolute size-3.5 transition-opacity', fade, working ? 'opacity-100' : 'opacity-0')} />
        </button>
        {/* The gradient edge: on while focused or working. Under reduced motion it stays still. */}
        <div
          aria-hidden
          data-slot="ai-border"
          data-flow={working && !reduced && visible}
          className={cn('ai-border pointer-events-none absolute inset-0 rounded-[inherit] opacity-0 transition-opacity group-focus-within/chat-input:opacity-100', fade, working && 'opacity-100')}
        />
      </div>
    </div>
  )
}
