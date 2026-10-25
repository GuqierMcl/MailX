import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { useId, useRef, useState, type ClipboardEvent, type KeyboardEvent, type ReactNode } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import { Cancel01Icon } from '@hugeicons/core-free-icons'
import { cn } from '@/lib/utils'
import { formatRecipient, mergeRecipients, parseRecipients, splitRecipients, parseRecipient, type Recipient } from './address-parse'

type RecipientFieldProps = {
  label: string
  value: Recipient[]
  onChange: (next: Recipient[]) => void
  disabled?: boolean
  autoFocus?: boolean
  /** Extra controls at the right edge of the row (the Cc / Bcc toggles). */
  trailing?: ReactNode
}

/**
 * Tokenized address input. Enter, comma, semicolon or leaving the field turns the
 * text into chips; pasting "Name <a@b>, c@d" splits at once; Backspace on an empty
 * input removes the last chip. An address that does not parse stays as an error
 * chip (and blocks sending) so nothing the user typed disappears.
 */
export function RecipientField({ label, value, onChange, disabled, autoFocus, trailing }: RecipientFieldProps) {
  useTranslation('composer')
  const [text, setText] = useState('')
  const inputRef = useRef<HTMLInputElement>(null)
  const id = useId()
  const invalidCount = value.filter((item) => !item.valid).length

  const commit = (incoming: Recipient[]) => {
    if (incoming.length) onChange(mergeRecipients(value, incoming))
  }

  const commitText = () => {
    const parsed = parseRecipients(text)
    if (text) setText('')
    commit(parsed)
  }

  const onInput = (next: string) => {
    const { segments, trailing: rest } = splitRecipients(next)
    if (segments.length === 0) {
      setText(next)
      return
    }
    commit(segments.flatMap((segment) => parseRecipient(segment) ?? []))
    setText(rest.trimStart())
  }

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.nativeEvent.isComposing) return
    if (event.key === 'Enter') {
      if (text.trim()) {
        // Cmd/Ctrl+Enter also sends (see SendButton): the typed address must be a chip by then.
        if (!(event.metaKey || event.ctrlKey)) event.preventDefault()
        commitText()
      }
    } else if (event.key === 'Backspace' && text === '' && value.length > 0) {
      event.preventDefault()
      onChange(value.slice(0, -1))
    }
  }

  const onPaste = (event: ClipboardEvent<HTMLInputElement>) => {
    const pasted = event.clipboardData.getData('text')
    if (!/[,;，；\n\r]/.test(pasted)) return
    event.preventDefault()
    commit(parseRecipients(text + pasted))
    setText('')
  }

  const edit = (index: number) => {
    const item = value[index]
    onChange(value.filter((_, position) => position !== index))
    setText(formatRecipient(item))
    inputRef.current?.focus()
  }

  return (
    <div
      data-slot="recipient-field"
      className="group/field flex items-start gap-2 rounded-lg px-2 py-1 transition-colors focus-within:bg-muted/50 hover:bg-muted/30"
      onMouseDown={(event) => {
        // Clicking the empty part of the row focuses the input, not the chips.
        if (event.target === event.currentTarget) {
          event.preventDefault()
          inputRef.current?.focus()
        }
      }}
    >
      <label htmlFor={id} className="w-14 shrink-0 pt-1.5 text-xs text-muted-foreground">{label}</label>
      <div className="flex min-w-0 flex-1 flex-wrap items-center gap-1">
        {value.map((item, index) => (
          <span
            key={`${item.address}-${index}`}
            data-invalid={!item.valid || undefined}
            title={item.valid ? item.address : t('composer:recipient.invalid', { address: item.address })}
            className={cn(
              'inline-flex max-w-full items-center gap-0.5 rounded-md py-0.5 pr-0.5 pl-2 text-xs',
              item.valid ? 'bg-secondary text-secondary-foreground' : 'text-destructive ring-1 ring-destructive/60 ring-inset bg-destructive/10',
            )}
          >
            <button
              type="button"
              disabled={disabled}
              onClick={() => edit(index)}
              aria-label={item.valid ? undefined : t('composer:recipient.editInvalid', { address: item.address })}
              className="min-w-0 truncate rounded-sm outline-hidden focus-visible:ring-2 focus-visible:ring-ring/50"
            >
              {item.valid ? item.name || item.address : item.address}
            </button>
            {!disabled && (
              <button
                type="button"
                aria-label={t('composer:recipient.remove', { address: item.address })}
                onClick={() => onChange(value.filter((_, position) => position !== index))}
                className="flex size-4 shrink-0 items-center justify-center rounded-sm opacity-60 outline-hidden hover:opacity-100 focus-visible:ring-2 focus-visible:ring-ring/50"
              >
                <HugeiconsIcon icon={Cancel01Icon} className="size-3" strokeWidth={2} />
              </button>
            )}
          </span>
        ))}
        <input
          id={id}
          ref={inputRef}
          value={text}
          disabled={disabled}
          autoFocus={autoFocus}
          aria-invalid={invalidCount > 0 || undefined}
          autoComplete="off"
          autoCapitalize="off"
          spellCheck={false}
          onChange={(event) => onInput(event.target.value)}
          onKeyDown={onKeyDown}
          onPaste={onPaste}
          onBlur={commitText}
          className="h-6 min-w-24 flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground disabled:cursor-not-allowed"
        />
      </div>
      {trailing && <div className="flex shrink-0 items-center gap-0.5 pt-0.5">{trailing}</div>}
    </div>
  )
}
