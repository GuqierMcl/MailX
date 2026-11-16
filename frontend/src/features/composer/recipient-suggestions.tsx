import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import type { ContactSuggestion } from '@/generated/accounts'
import { cn } from '@/lib/utils'

/** Where `query` matches: a word start (or anywhere, for CJK), case-insensitively; -1 when it does not. */
function matchAt(text: string, query: string) {
  const lower = text.toLowerCase()
  const term = query.toLowerCase()
  if (!term) return -1
  for (let at = lower.indexOf(term); at >= 0; at = lower.indexOf(term, at + 1)) {
    if (at === 0 || /[\s.\-_@<"']/.test(lower[at - 1]) || /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]/u.test(term)) return at
  }
  return -1
}

function Highlighted({ text, query }: { text: string; query: string }) {
  const at = matchAt(text, query.trim())
  if (at < 0) return <>{text}</>
  const end = at + query.trim().length
  return <>{text.slice(0, at)}<strong className="font-semibold">{text.slice(at, end)}</strong>{text.slice(end)}</>
}

type SuggestionListProps = {
  id: string
  /** The row's text area: the list starts under it, aligned with the chips. */
  anchor: HTMLElement | null
  items: ContactSuggestion[]
  active: number
  query: string
  optionId: (index: number) => string
  onHover: (index: number) => void
  onPick: (item: ContactSuggestion) => void
}

/**
 * The list under a recipient row. It is portalled to the page so the island's
 * overflow cannot clip it, and fixed below the anchor so it never covers the
 * input. Pressing an item keeps focus in the input (so the field does not blur
 * and close the list first); the click then picks it. Same surface as the
 * sender picker's list.
 */
export function SuggestionList({ id, anchor, items, active, query, optionId, onHover, onPick }: SuggestionListProps) {
  const [style, setStyle] = useState<CSSProperties | null>(null)
  const list = useRef<HTMLDivElement>(null)

  useLayoutEffect(() => {
    if (!anchor) return
    const place = () => {
      const rect = anchor.getBoundingClientRect()
      // The anchor is the row's text area; the row's own padding sits below it.
      const top = rect.bottom + 8
      setStyle({ top, left: rect.left, width: Math.min(Math.max(rect.width, 240), 360), maxHeight: Math.max(window.innerHeight - top - 8, 96) })
    }
    place()
    window.addEventListener('resize', place)
    return () => window.removeEventListener('resize', place)
  }, [anchor, items.length])

  useEffect(() => {
    list.current?.querySelector(`[id="${CSS.escape(optionId(active))}"]`)?.scrollIntoView?.({ block: 'nearest' })
  }, [active, optionId])

  if (!anchor || !style) return null
  return createPortal(
    <div
      ref={list}
      id={id}
      role="listbox"
      data-slot="recipient-suggestions"
      style={style}
      onPointerDown={(event) => event.preventDefault()}
      className="fixed z-50 animate-in overflow-y-auto rounded-lg bg-popover p-1 text-popover-foreground shadow-md ring-1 ring-foreground/10 duration-100 fade-in-0 motion-reduce:animate-none"
    >
      {items.map((item, index) => (
        <Option key={item.address} id={optionId(index)} selected={index === active} onHover={() => onHover(index)} onPick={() => onPick(item)}>
          {item.name ? (
            <>
              <span className="truncate text-sm"><Highlighted text={item.name} query={query} /></span>
              <span className="truncate text-xs text-muted-foreground group-aria-selected/option:text-accent-foreground/80"><Highlighted text={item.address} query={query} /></span>
            </>
          ) : <span className="truncate text-sm"><Highlighted text={item.address} query={query} /></span>}
        </Option>
      ))}
    </div>,
    document.body,
  )
}

function Option({ id, selected, onHover, onPick, children }: { id: string; selected: boolean; onHover: () => void; onPick: () => void; children: ReactNode }) {
  return (
    <div
      id={id}
      role="option"
      aria-selected={selected}
      onPointerMove={onHover}
      onClick={onPick}
      className={cn('group/option flex min-w-0 cursor-default flex-col gap-0.5 rounded-md px-2 py-1.5 select-none', selected && 'bg-accent text-accent-foreground')}
    >
      {children}
    </div>
  )
}
