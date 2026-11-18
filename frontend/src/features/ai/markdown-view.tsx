import { memo, useCallback, useEffect, useMemo, useRef, useState, type CSSProperties, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { CheckmarkSquare02Icon, Copy01Icon, SquareIcon, Tick02Icon } from '@hugeicons/core-free-icons'
import { t } from '@/i18n'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { AiMark } from './ai-mark'
import { RefView } from './ref-cards'
import { useDocumentVisible } from './use-document-visible'
import { blocksOf, createMarkdownReader, type Inline, type MarkdownBlock, type MarkdownPiece, type TableAlign } from './markdown'

type LinkHandler = (url: string) => void
/** The streaming caret: shown while the reply is written, fading out once it is done. */
type CaretState = 'none' | 'live' | 'leaving'

/** How long the caret takes to fade out (index.css `.ai-caret[data-state="leaving"]`, `--motion-medium`). */
const CARET_FADE_MS = 200
/** How long the copy button shows its check after copying. */
const COPIED_MS = 1500

function Inlines({ nodes, onOpenLink }: { nodes: Inline[]; onOpenLink: LinkHandler }): ReactNode {
  return nodes.map((node, index) => {
    switch (node.type) {
      case 'text': return node.text
      case 'br': return <br key={index} />
      case 'code': return <code key={index} className="rounded-sm bg-muted px-1 py-0.5 font-mono text-[0.85em]">{node.text}</code>
      case 'strong': return <strong key={index} className="font-semibold"><Inlines nodes={node.children} onOpenLink={onOpenLink} /></strong>
      case 'em': return <em key={index}><Inlines nodes={node.children} onOpenLink={onOpenLink} /></em>
      case 'del': return <del key={index} className="text-muted-foreground"><Inlines nodes={node.children} onOpenLink={onOpenLink} /></del>
      case 'ref': return <RefView key={index} kind={node.kind} id={node.id} label={node.label} block={false} />
      case 'link': return (
        // The address is the tooltip (the full URL on hover); the click goes to the system browser, never in-app.
        <a
          key={index}
          href={node.href}
          title={node.href}
          rel="noreferrer noopener"
          className="rounded-sm text-foreground underline decoration-foreground/30 underline-offset-2 outline-none hover:decoration-foreground focus-visible:ring-2 focus-visible:ring-ring"
          onClick={(event) => { event.preventDefault(); onOpenLink(node.href) }}
        >
          <Inlines nodes={node.children} onOpenLink={onOpenLink} />
        </a>
      )
    }
  })
}

/** The AI sparkle at the end of the text being written, breathing gently; static under reduced motion. */
function StreamCaret({ state }: { state: Exclude<CaretState, 'none'> }) {
  const visible = useDocumentVisible()
  return <span aria-hidden data-slot="stream-caret" data-state={state} data-paused={!visible} className="ai-caret"><AiMark className="size-full" /></span>
}

function CodeBlock({ lang, text, className }: { lang: string | null; text: string; className?: string }) {
  useTranslation()
  const [copied, setCopied] = useState(false)
  useEffect(() => {
    if (!copied) return
    const timer = setTimeout(() => setCopied(false), COPIED_MS)
    return () => clearTimeout(timer)
  }, [copied])
  const copy = () => { void navigator.clipboard.writeText(text).then(() => setCopied(true), () => undefined) }
  const icon = 'absolute inset-0 size-3.5 transition-opacity duration-150 ease-out motion-reduce:transition-none'
  return (
    <div data-slot="markdown-code" className={cn('group/code relative rounded-lg bg-muted/60', className)}>
      {lang && <div className="px-3 pt-1.5 font-mono text-xs text-muted-foreground select-none">{lang}</div>}
      <pre className={cn('overflow-x-auto px-3 font-mono text-[0.85em] leading-relaxed whitespace-pre', lang ? 'pt-0.5 pb-2' : 'py-2 pr-9')}><code>{text}</code></pre>
      <Button
        type="button"
        variant="ghost"
        size="icon-sm"
        aria-label={copied ? t('ai:chat.codeCopied') : t('ai:chat.copyCode')}
        title={copied ? t('ai:chat.codeCopied') : t('ai:chat.copyCode')}
        onClick={copy}
        className="absolute top-1 right-1 text-muted-foreground opacity-0 transition-opacity duration-150 group-hover/code:opacity-100 group-focus-within/code:opacity-100 focus-visible:opacity-100"
      >
        <span className="relative size-3.5">
          <HugeiconsIcon icon={Copy01Icon} aria-hidden strokeWidth={2} className={cn(icon, copied && 'opacity-0')} />
          <HugeiconsIcon icon={Tick02Icon} aria-hidden strokeWidth={2} className={cn(icon, !copied && 'opacity-0')} />
        </span>
      </Button>
    </div>
  )
}

const cellAlign = (align: TableAlign): CSSProperties | undefined => (align ? { textAlign: align } : undefined)

function TableBlock({ block, onOpenLink, className }: { block: Extract<MarkdownBlock, { type: 'table' }>; onOpenLink: LinkHandler; className?: string }) {
  return (
    // As wide as its content, scrolling inside the column instead of widening the reply. No grid lines: rows are told apart by a faint tint.
    <div data-slot="markdown-table" className={cn('w-fit max-w-full overflow-x-auto rounded-lg', className)}>
      <table className="border-collapse text-sm leading-snug tabular-nums wrap-break-word">
        <thead className="bg-muted/60">
          <tr>{block.header.map((cell, at) => <th key={at} scope="col" className="px-3 py-1.5 text-left align-bottom font-medium" style={cellAlign(block.align[at])}><Inlines nodes={cell} onOpenLink={onOpenLink} /></th>)}</tr>
        </thead>
        <tbody>
          {block.rows.map((row, at) => (
            <tr key={at} className="even:bg-muted/30">
              {row.map((cell, index) => <td key={index} className="px-3 py-1.5 text-left align-top" style={cellAlign(block.align[index])}><Inlines nodes={cell} onOpenLink={onOpenLink} /></td>)}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

const HEADING_CLASS: Record<number, string> = {
  1: 'mt-1.5 text-[1.0625rem] first:mt-0',
  2: 'mt-1 text-base first:mt-0',
}

/**
 * Blocks as elements. `caret` goes at the end of the last block's text, or on its own line after a
 * block that has no running text at its end (code, table, rule, card). `blockClass` is given to each
 * top-level element (the entrance fade).
 */
function Blocks({ blocks, onOpenLink, caret = null, blockClass }: { blocks: MarkdownBlock[]; onOpenLink: LinkHandler; caret?: ReactNode; blockClass?: string }): ReactNode {
  const out = blocks.map((block, index) => {
    const end = caret !== null && index === blocks.length - 1 ? caret : null
    switch (block.type) {
      case 'paragraph': return <p key={index} className={blockClass}><Inlines nodes={block.children} onOpenLink={onOpenLink} />{end}</p>
      case 'heading': return <p key={index} data-level={block.level} className={cn('font-semibold', HEADING_CLASS[block.level], blockClass)}><Inlines nodes={block.children} onOpenLink={onOpenLink} />{end}</p>
      case 'ref': return <RefView key={index} kind={block.kind} id={block.id} label={block.label} block />
      case 'code': return <CodeBlock key={index} lang={block.lang} text={block.text} className={blockClass} />
      case 'table': return <TableBlock key={index} block={block} onOpenLink={onOpenLink} className={blockClass} />
      case 'rule': return (
        // Not a divider across the column (ui-design.md): a short bar of the AI gradient, one of its quiet accents.
        <div key={index} role="separator" className={cn('flex justify-center py-1.5', blockClass)}>
          <span className="h-px w-12 rounded-full bg-(image:--ai-gradient) opacity-50" />
        </div>
      )
      case 'quote': return <blockquote key={index} className={cn('flex flex-col gap-2 border-l-2 border-foreground/15 pl-3 text-muted-foreground', blockClass)}><Blocks blocks={block.children} onOpenLink={onOpenLink} caret={end} /></blockquote>
      case 'list': {
        const Tag = block.ordered ? 'ol' : 'ul'
        return (
          <Tag key={index} start={block.ordered && block.start !== 1 ? block.start : undefined} className={cn('flex flex-col pl-5', block.loose ? 'gap-2' : 'gap-1', block.ordered ? 'list-decimal' : 'list-disc', blockClass)}>
            {block.items.map((item, at) => (
              // A list item keeps `display: list-item` (a flex item would lose its number or bullet); its blocks are spaced by margins.
              <li key={at} className={cn('relative pl-0.5 marker:text-muted-foreground', block.loose ? 'space-y-2' : 'space-y-1', item.checked !== null && 'list-none')}>
                {item.checked !== null && (
                  <HugeiconsIcon
                    icon={item.checked ? CheckmarkSquare02Icon : SquareIcon}
                    role="img"
                    aria-label={item.checked ? t('ai:chat.taskDone') : t('ai:chat.taskOpen')}
                    strokeWidth={2}
                    className="absolute top-[0.3em] -left-4.5 size-[1em] text-muted-foreground"
                  />
                )}
                <Blocks blocks={item.blocks} onOpenLink={onOpenLink} caret={at === block.items.length - 1 ? end : null} />
              </li>
            ))}
          </Tag>
        )
      }
    }
  })
  const last = blocks[blocks.length - 1]
  // Code, a table, a rule or a card ends without running text: the caret takes a line of its own.
  if (caret !== null && (!last || last.type === 'code' || last.type === 'table' || last.type === 'rule' || last.type === 'ref')) {
    out.push(<span key="caret" className="block leading-none">{caret}</span>)
  }
  return out
}

type PieceProps = { piece: MarkdownPiece; caret: CaretState; fresh: boolean; onOpenLink: LinkHandler }

/**
 * One top-level block. It is read and drawn again only when its source, its open state or the caret
 * changes: finished blocks of a streaming reply stay as they are.
 */
const Piece = memo(function Piece({ piece, caret, fresh, onOpenLink }: PieceProps) {
  const blocks = useMemo(() => blocksOf(piece.token), [piece.token])
  return (
    <Blocks
      blocks={blocks}
      onOpenLink={onOpenLink}
      caret={caret === 'none' ? null : <StreamCaret state={caret} />}
      blockClass={fresh ? 'animate-md-block-in motion-reduce:animate-none' : undefined}
    />
  )
}, (before, after) => before.piece.raw === after.piece.raw && before.piece.open === after.piece.open && before.caret === after.caret && before.fresh === after.fresh)

/** The caret's state: live while streaming, then a short fade-out, then gone. A reply that was never streamed here has none. */
function useCaret(streaming: boolean): CaretState {
  const [streamedHere] = useState(streaming)
  const [gone, setGone] = useState(false)
  useEffect(() => {
    if (streaming || !streamedHere) return
    const timer = setTimeout(() => setGone(true), CARET_FADE_MS)
    return () => clearTimeout(timer)
  }, [streaming, streamedHere])
  if (streaming) return 'live'
  return streamedHere && !gone ? 'leaving' : 'none'
}

/**
 * Renders a reply's text as the Markdown of `markdown.ts`, as plain React elements: no HTML is ever
 * inserted. While `streaming`, only the source from the last block on is lexed again; finished blocks
 * keep their source and neither parse nor render again. A block that appears while the reply streams
 * fades in; blocks that were there when the reply was shown do not.
 */
export const Markdown = memo(function Markdown({ text, streaming = false, onOpenLink, className }: { text: string; streaming?: boolean; onOpenLink: LinkHandler; className?: string }) {
  const [read] = useState(createMarkdownReader)
  const pieces = useMemo(() => read(text, streaming), [read, text, streaming])
  // Blocks from this index on arrived while streaming after the reply appeared.
  const [firstFresh] = useState(() => (streaming ? pieces.length : Number.POSITIVE_INFINITY))
  const caret = useCaret(streaming)
  // The handler may be a new function on every render; the blocks get one that never changes.
  const handler = useRef(onOpenLink)
  useEffect(() => { handler.current = onOpenLink })
  const openLink = useCallback((url: string) => handler.current(url), [])
  if (pieces.length === 0) return null
  return (
    <div data-slot="markdown" className={cn('flex flex-col gap-2.5 text-[0.9375rem] leading-relaxed wrap-anywhere', className)}>
      {pieces.map((piece, index) => (
        <Piece key={index} piece={piece} caret={index === pieces.length - 1 ? caret : 'none'} fresh={index >= firstFresh} onOpenLink={openLink} />
      ))}
    </div>
  )
})
