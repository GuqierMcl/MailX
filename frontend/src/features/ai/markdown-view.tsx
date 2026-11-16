import { memo, useMemo, type ReactNode } from 'react'
import { cn } from '@/lib/utils'
import { RefView } from './ref-cards'
import { parseBlock, splitBlocks, type Inline, type MarkdownBlock } from './markdown'

type LinkHandler = (url: string) => void

function Inlines({ nodes, onOpenLink }: { nodes: Inline[]; onOpenLink: LinkHandler }): ReactNode {
  return nodes.map((node, index) => {
    switch (node.type) {
      case 'text': return node.text
      case 'br': return <br key={index} />
      case 'code': return <code key={index} className="rounded-sm bg-muted px-1 py-0.5 font-mono text-[0.85em]">{node.text}</code>
      case 'strong': return <strong key={index} className="font-semibold"><Inlines nodes={node.children} onOpenLink={onOpenLink} /></strong>
      case 'em': return <em key={index}><Inlines nodes={node.children} onOpenLink={onOpenLink} /></em>
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

function Blocks({ blocks, onOpenLink }: { blocks: MarkdownBlock[]; onOpenLink: LinkHandler }): ReactNode {
  return blocks.map((block, index) => {
    switch (block.type) {
      case 'paragraph': return <p key={index}><Inlines nodes={block.children} onOpenLink={onOpenLink} /></p>
      case 'heading': return <p key={index} className="font-semibold"><Inlines nodes={block.children} onOpenLink={onOpenLink} /></p>
      case 'ref': return <RefView key={index} kind={block.kind} id={block.id} label={block.label} block />
      case 'code': return <pre key={index} className="overflow-x-auto rounded-lg bg-muted/60 px-3 py-2 font-mono text-[0.85em] leading-relaxed whitespace-pre"><code>{block.text}</code></pre>
      case 'quote': return <blockquote key={index} className="flex flex-col gap-2 border-l-2 border-foreground/15 pl-3 text-muted-foreground"><Blocks blocks={block.children} onOpenLink={onOpenLink} /></blockquote>
      case 'list': {
        const Tag = block.ordered ? 'ol' : 'ul'
        return (
          <Tag key={index} start={block.ordered ? block.start : undefined} className={cn('flex flex-col gap-1 pl-5', block.ordered ? 'list-decimal' : 'list-disc')}>
            {block.items.map((item, at) => <li key={at} className="flex flex-col gap-1 pl-0.5 marker:text-muted-foreground"><Blocks blocks={item} onOpenLink={onOpenLink} /></li>)}
          </Tag>
        )
      }
    }
  })
}

/** One finished or growing piece. The parse is kept per source, so a finished piece is never redone. */
const Piece = memo(function Piece({ source, open, onOpenLink }: { source: string; open: boolean; onOpenLink: LinkHandler }) {
  const blocks = useMemo(() => parseBlock(source, open), [source, open])
  return <Blocks blocks={blocks} onOpenLink={onOpenLink} />
})

/**
 * Renders a reply's text as the restricted Markdown of `markdown.ts`, as plain React elements: no HTML
 * is ever inserted. While `streaming`, only the last piece changes between updates; the others keep
 * their string, so they neither parse nor render again.
 */
export const Markdown = memo(function Markdown({ text, streaming = false, onOpenLink, className }: { text: string; streaming?: boolean; onOpenLink: LinkHandler; className?: string }) {
  const pieces = useMemo(() => splitBlocks(text), [text])
  if (pieces.length === 0) return null
  return (
    <div data-slot="markdown" className={cn('flex flex-col gap-2.5 text-[0.9375rem] leading-relaxed wrap-anywhere', className)}>
      {pieces.map((source, index) => <Piece key={index} source={source} open={streaming && index === pieces.length - 1} onOpenLink={onOpenLink} />)}
    </div>
  )
})
