import type { ReactNode } from 'react'
import { parseNotes, truncateNotes, type Inline } from './notes-markdown'

function inlineNodes(nodes: Inline[], onOpenLink?: (url: string) => void, key = ''): ReactNode[] {
  return nodes.map((node, index) => {
    const id = `${key}${index}`
    switch (node.kind) {
      case 'text':
        return <span key={id}>{node.text}</span>
      case 'code':
        return <code key={id} className="rounded bg-muted px-1 py-0.5 font-mono text-[0.9em]">{node.text}</code>
      case 'strong':
        return <strong key={id}>{inlineNodes(node.children, onOpenLink, `${id}-`)}</strong>
      case 'link':
        return (
          <button
            key={id}
            type="button"
            className="inline text-left font-medium text-primary underline-offset-2 hover:underline focus-visible:underline focus-visible:outline-none"
            onClick={() => onOpenLink?.(node.url)}
          >
            {node.text}
          </button>
        )
    }
  })
}

const HEADING_CLASS = {
  1: 'text-base font-semibold',
  2: 'text-sm font-semibold',
  3: 'text-sm font-medium',
} as const

/** Rendered release notes. `truncatedHint` follows the text when it was cut. */
export function UpdateNotes({ notes, onOpenLink, truncatedHint }: {
  notes: string
  onOpenLink?: (url: string) => void
  truncatedHint?: ReactNode
}) {
  const { text, truncated } = truncateNotes(notes)
  const blocks = parseNotes(text)
  return (
    <div className="flex flex-col gap-3 text-sm leading-relaxed">
      {blocks.map((block, index) => {
        const key = `block-${index}`
        switch (block.kind) {
          case 'heading':
            return (
              <p key={key} className={HEADING_CLASS[block.level]}>
                {inlineNodes(block.children, onOpenLink, `${key}-`)}
              </p>
            )
          case 'paragraph':
            return <p key={key} className="whitespace-pre-line text-muted-foreground">{inlineNodes(block.children, onOpenLink, `${key}-`)}</p>
          case 'list':
            return (
              <ul key={key} className="ml-4 list-disc space-y-1 text-muted-foreground marker:text-muted-foreground/60">
                {block.items.map((item, itemIndex) => (
                  <li key={itemIndex}>{inlineNodes(item, onOpenLink, `${key}-${itemIndex}-`)}</li>
                ))}
              </ul>
            )
          case 'quote':
            return (
              <blockquote key={key} className="border-l-2 pl-3 text-muted-foreground italic">
                {block.lines.map((line, lineIndex) => (
                  <p key={lineIndex}>{inlineNodes(line, onOpenLink, `${key}-${lineIndex}-`)}</p>
                ))}
              </blockquote>
            )
        }
      })}
      {truncated && truncatedHint && <p className="text-xs text-muted-foreground">{truncatedHint}</p>}
    </div>
  )
}
