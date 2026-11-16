import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { useCallback, useEffect, useId, useImperativeHandle, useRef, useState, type ClipboardEvent, type FocusEvent, type KeyboardEvent, type ReactNode, type Ref } from 'react'
import { ContextMenu as ContextMenuPrimitive } from '@base-ui/react/context-menu'
import { useDraggable, useDroppable } from '@dnd-kit/react'
import { HugeiconsIcon } from '@hugeicons/react'
import { ArrowRight02Icon, Cancel01Icon, Copy01Icon } from '@hugeicons/core-free-icons'
import { ContextMenuContent, ContextMenuItem, ContextMenuSeparator } from '@/components/ui/context-menu'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { anchorContextMenuAt } from '@/lib/context-menu-anchor'
import { cn } from '@/lib/utils'
import { formatRecipient, isValidAddress, mergeRecipients, parseRecipients, splitRecipients, parseRecipient, type Recipient } from './address-parse'
import { SuggestionList } from './recipient-suggestions'
import { useContactSuggestions, type SuggestContacts } from './use-contact-suggestions'
import type { ContactSuggestion } from '@/generated/accounts'

/** The three recipient rows; chips move between them. */
export type RecipientRow = 'to' | 'cc' | 'bcc'

/** What a dragged chip carries, and what a row (or a collapsed row's toggle) accepts. */
export type RecipientDragData = { row: RecipientRow; index: number }
export type RecipientDropData = { row: RecipientRow }

const MOVE_LABEL: Record<RecipientRow, 'composer:recipient.moveToTo' | 'composer:recipient.moveToCc' | 'composer:recipient.moveToBcc'> = {
  to: 'composer:recipient.moveToTo', cc: 'composer:recipient.moveToCc', bcc: 'composer:recipient.moveToBcc',
}

export type RecipientFieldHandle = { focus: () => void; /** Whether focus is anywhere in the row. */ hasFocus: () => boolean }

type RecipientFieldProps = {
  label: string
  value: Recipient[]
  onChange: (next: Recipient[]) => void
  disabled?: boolean
  autoFocus?: boolean
  /** Extra controls at the right edge of the row (the Cc / Bcc toggles). */
  trailing?: ReactNode
  /**
   * Which row this is. With it the row accepts dragged chips, its chips can be
   * dragged, and their menu offers moving to the other rows through `onMove`.
   */
  row?: RecipientRow
  onMove?: (index: number, to: RecipientRow) => void
  /** Focus entered (true) or left (false) the row as a whole. */
  onFocusChange?: (focused: boolean) => void
  fieldRef?: Ref<RecipientFieldHandle>
  /** Address suggestions as the user types (sending.md section 17.4). */
  suggest?: SuggestContacts
  /** Lowercase addresses already in any recipient row; they are never suggested. */
  exclude?: ReadonlySet<string>
}

const NONE: ReadonlySet<string> = new Set()

/**
 * Tokenized address input. Enter, comma, semicolon or leaving the field turns the
 * text into chips; pasting "Name <a@b>, c@d" splits at once; Backspace on an empty
 * input removes the last chip. An address that does not parse stays as an error
 * chip (and blocks sending) so nothing the user typed disappears.
 *
 * Chips are separate components so the input stays the row's one text control;
 * suggestions (sending.md section 17.4) attach to that input.
 */
export function RecipientField({ label, value, onChange, disabled, autoFocus, trailing, row, onMove, onFocusChange, fieldRef, suggest, exclude = NONE }: RecipientFieldProps) {
  useTranslation('composer')
  const [text, setText] = useState('')
  const inputRef = useRef<HTMLInputElement>(null)
  const id = useId()
  const invalidCount = value.filter((item) => !item.valid).length
  const { ref: dropRef, isDropTarget } = useDroppable<RecipientDropData>({ id: `recipient-row-${row ?? id}`, data: { row: row ?? 'to' }, disabled: !row || disabled })
  const rowRef = useRef<HTMLDivElement | null>(null)
  const [chipArea, setChipArea] = useState<HTMLDivElement | null>(null)
  const composing = useRef(false)
  const suggestions = useContactSuggestions(suggest, exclude)
  const listId = `${id}-suggestions`
  const optionId = useCallback((index: number) => `${id}-suggestion-${index}`, [id])
  useImperativeHandle(fieldRef, () => ({
    focus: () => inputRef.current?.focus(),
    hasFocus: () => rowRef.current?.contains(document.activeElement) ?? false,
  }), [])

  const commit = (incoming: Recipient[]) => {
    if (incoming.length) onChange(mergeRecipients(value, incoming))
  }

  const commitText = () => {
    suggestions.close()
    const parsed = parseRecipients(text)
    if (text) setText('')
    commit(parsed)
  }

  const onInput = (next: string) => {
    const { segments, trailing: rest } = splitRecipients(next)
    if (segments.length === 0) {
      setText(next)
      // Mid-composition text is not what the user means yet; ask once it ends.
      if (!composing.current) suggestions.request(next)
      return
    }
    commit(segments.flatMap((segment) => parseRecipient(segment) ?? []))
    setText(rest.trimStart())
    suggestions.request(rest)
  }

  const pick = (item: ContactSuggestion) => {
    suggestions.close()
    setText('')
    commit([{ name: item.name, address: item.address, valid: isValidAddress(item.address) }])
    inputRef.current?.focus()
  }

  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    // keyCode 229: some WebViews report the key that ends a composition without `isComposing`.
    if (event.nativeEvent.isComposing || composing.current || event.keyCode === 229) return
    if (suggestions.open) {
      const plain = !event.metaKey && !event.ctrlKey && !event.altKey
      if (plain && (event.key === 'ArrowDown' || event.key === 'ArrowUp')) {
        event.preventDefault()
        suggestions.move(event.key === 'ArrowDown' ? 1 : -1)
        return
      }
      if (plain && !event.shiftKey && (event.key === 'Enter' || event.key === 'Tab')) {
        event.preventDefault()
        pick(suggestions.items[suggestions.active])
        return
      }
      if (event.key === 'Escape') {
        // Only the list closes: the typed text stays, and nothing else (the send button's confirmation) sees this Esc.
        event.preventDefault()
        event.stopPropagation()
        suggestions.close()
        return
      }
    }
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

  const remove = (index: number) => onChange(value.filter((_, position) => position !== index))

  // Focus moving between the input and the chips is not leaving the row.
  const focusLeaves = (event: FocusEvent<HTMLDivElement>) => !event.currentTarget.contains(event.relatedTarget as Node | null)

  return (
    <div
      ref={(element) => { rowRef.current = element; dropRef(element) }}
      data-slot="recipient-field"
      data-drop-target={isDropTarget || undefined}
      className="group/field flex items-start gap-2 rounded-lg px-2 py-1 transition-colors focus-within:bg-muted/50 hover:bg-muted/30 data-drop-target:bg-muted/60 motion-reduce:transition-none"
      onFocus={(event) => { if (focusLeaves(event)) onFocusChange?.(true) }}
      onBlur={(event) => { if (focusLeaves(event)) onFocusChange?.(false) }}
      onMouseDown={(event) => {
        // Clicking the empty part of the row (or of the chip area) focuses the input, not the chips.
        if (event.target === event.currentTarget || (event.target as HTMLElement).dataset.slot === 'recipient-chips') {
          event.preventDefault()
          inputRef.current?.focus()
        }
      }}
    >
      <label htmlFor={id} className="w-14 shrink-0 text-xs leading-6 text-muted-foreground">{label}</label>
      <div ref={setChipArea} data-slot="recipient-chips" className="flex min-w-0 flex-1 flex-wrap items-center gap-1">
        {value.map((item, index) => (
          <RecipientChip
            key={`${item.address}-${index}`}
            item={item}
            index={index}
            row={row}
            disabled={disabled}
            onEdit={() => edit(index)}
            onRemove={() => remove(index)}
            onMove={onMove && ((to) => onMove(index, to))}
          />
        ))}
        {/* Unfocused it takes only what is left of the last line, so full chips never leave an empty
            line under them; focused it keeps room to type and wraps only when the line is truly full. */}
        <input
          id={id}
          ref={inputRef}
          value={text}
          disabled={disabled}
          autoFocus={autoFocus}
          aria-invalid={invalidCount > 0 || undefined}
          role="combobox"
          aria-autocomplete="list"
          aria-expanded={suggestions.open}
          aria-controls={suggestions.open ? listId : undefined}
          aria-activedescendant={suggestions.open ? optionId(suggestions.active) : undefined}
          autoComplete="off"
          autoCapitalize="off"
          spellCheck={false}
          onChange={(event) => onInput(event.target.value)}
          onKeyDown={onKeyDown}
          onPaste={onPaste}
          onBlur={commitText}
          onCompositionStart={() => { composing.current = true }}
          onCompositionEnd={(event) => {
            composing.current = false
            suggestions.request(splitRecipients(event.currentTarget.value).trailing)
          }}
          className="h-6 w-0 min-w-0 flex-1 basis-0 bg-transparent text-sm focus:min-w-16 focus:basis-16 outline-none placeholder:text-muted-foreground disabled:cursor-not-allowed"
        />
      </div>
      {trailing && <div className="flex h-6 shrink-0 items-center gap-0.5">{trailing}</div>}
      {suggestions.open && (
        <SuggestionList id={listId} anchor={chipArea} items={suggestions.items} active={suggestions.active} query={text}
          optionId={optionId} onHover={suggestions.setActive} onPick={pick} />
      )}
    </div>
  )
}

type RecipientChipProps = {
  item: Recipient
  index: number
  row?: RecipientRow
  disabled?: boolean
  onEdit: () => void
  onRemove: () => void
  onMove?: (to: RecipientRow) => void
}

/**
 * One address. Click edits it, the × removes it; a drag (pointer only, after a
 * few pixels so a click stays a click) moves it to another row, and the context
 * menu (right click, the menu key or Shift+F10) offers the same by keyboard.
 */
function RecipientChip({ item, index, row, disabled, onEdit, onRemove, onMove }: RecipientChipProps) {
  const chip = useRef<HTMLSpanElement | null>(null)
  const draggedAt = useRef(0)
  const { ref, handleRef, isDragging } = useDraggable<RecipientDragData>({
    id: `recipient-${row ?? 'none'}-${index}-${item.address}`,
    data: { row: row ?? 'to', index },
    disabled: !row || disabled,
  })
  // A drag that ends over the chip must not also start editing it.
  useEffect(() => { if (!isDragging) return; return () => { draggedAt.current = Date.now() } }, [isDragging])
  const full = item.valid ? formatRecipient(item) : t('composer:recipient.invalid', { address: item.address })
  const targets = row && onMove ? (['to', 'cc', 'bcc'] as const).filter((target) => target !== row) : []

  return (
    <ContextMenuPrimitive.Root disabled={disabled}>
      <ContextMenuPrimitive.Trigger
        render={<span ref={(element: HTMLSpanElement | null) => { chip.current = element; ref(element) }} />}
        data-invalid={!item.valid || undefined}
        data-dragging={isDragging || undefined}
        className={cn(
          'inline-flex h-5 max-w-full items-center gap-0.5 rounded-md pr-0.5 pl-2 text-xs select-none',
          item.valid ? 'bg-secondary text-secondary-foreground' : 'bg-destructive/10 text-destructive ring-1 ring-destructive/60 ring-inset',
          'data-dragging:shadow-md',
        )}
        onContextMenu={(event) => { if (chip.current) anchorContextMenuAt(event, chip.current) }}
      >
        <Tooltip disabled={isDragging}>
          <TooltipTrigger
            render={(
              <button
                ref={handleRef}
                type="button"
                disabled={disabled}
                onClick={() => { if (Date.now() - draggedAt.current > 300) onEdit() }}
                aria-label={item.valid ? undefined : t('composer:recipient.editInvalid', { address: item.address })}
                className="min-w-0 truncate rounded-sm outline-hidden focus-visible:ring-2 focus-visible:ring-ring/50"
              />
            )}
          >
            {item.valid ? item.name || item.address : item.address}
          </TooltipTrigger>
          <TooltipContent>{full}</TooltipContent>
        </Tooltip>
        {!disabled && (
          <button
            type="button"
            aria-label={t('composer:recipient.remove', { address: item.address })}
            onClick={onRemove}
            className="flex size-4 shrink-0 items-center justify-center rounded-sm opacity-60 outline-hidden hover:opacity-100 focus-visible:ring-2 focus-visible:ring-ring/50"
          >
            <HugeiconsIcon icon={Cancel01Icon} className="size-3" strokeWidth={2} />
          </button>
        )}
      </ContextMenuPrimitive.Trigger>
      <ContextMenuContent aria-label={t('composer:recipient.menu', { address: item.address })} className="min-w-36">
        {targets.map((target) => (
          <ContextMenuItem key={target} onClick={() => onMove?.(target)}>
            <HugeiconsIcon icon={ArrowRight02Icon} />
            {t(MOVE_LABEL[target])}
          </ContextMenuItem>
        ))}
        {targets.length > 0 && <ContextMenuSeparator />}
        <ContextMenuItem onClick={() => void navigator.clipboard?.writeText(item.address).catch(() => undefined)}>
          <HugeiconsIcon icon={Copy01Icon} />
          {t('composer:recipient.copy')}
        </ContextMenuItem>
        <ContextMenuItem variant="destructive" onClick={onRemove}>
          <HugeiconsIcon icon={Cancel01Icon} />
          {t('composer:recipient.removeShort')}
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenuPrimitive.Root>
  )
}
