import { useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from 'react'
import { createPortal } from 'react-dom'
import { useTranslation } from 'react-i18next'
import { useQuery } from '@tanstack/react-query'
import { useEditorState } from '@tiptap/react'
import type { Editor } from '@tiptap/core'
import { HugeiconsIcon } from '@hugeicons/react'
import { ArrowDown01Icon, ArrowLeft01Icon, ArrowRight01Icon } from '@hugeicons/core-free-icons'
import { t } from '@/i18n'
import { MOD_KEY } from '@/features/mail/shortcuts'
import { errorText } from '@/i18n/errors'
import { Button } from '@/components/ui/button'
import {
  DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuSub,
  DropdownMenuSubContent, DropdownMenuSubTrigger, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Select, SelectContent, SelectItem, SelectSeparator, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { AiConsentDialog } from '@/features/ai/ai-consent-dialog'
import { AiMark } from '@/features/ai/ai-mark'
import { useAssistantRun, type RunApi } from '@/features/ai/use-assistant-run'
import { useDocumentVisible } from '@/features/ai/use-document-visible'
import { nativeLanguageName, TRANSLATION_TARGETS } from '@/features/mail/translation/languages'
import { useSettings } from '@/features/settings/settings-queries'
import { allowedMailLink } from '../mail-body-editor-schema'
import type { ComposeTask, DraftKind, ErrorCode, RewriteStyle } from '@/generated/accounts'
import { usePrefersReducedMotion } from '@/hooks/use-reduced-motion'
import type { MailxClient } from '@/lib/mailx-client'
import { cn } from '@/lib/utils'
import { answerHtml, answerText, blockEnd, diff, emptyBlock, hasImage, ownEnd, ownRange, rangeMarkdown, type DocRange } from './compose-ai'
import { composeAiKey, composeAiPlugin, type ComposeAiMarks, type ComposeAiMeta } from './compose-ai-plugin'

type Style = Exclude<RewriteStyle['type'], 'translate' | 'custom'>
const STYLES: Style[] = ['polish', 'formal', 'friendly', 'concise', 'expand']
const styleLabel = (style: Style) => t(`composer:ai.style.${style}`)
const AUTO = 'auto'

/** What the card is about. `replace` is the text the answer takes the place of; `null` inserts at `anchor`. */
type Job = {
  task: ComposeTask | null
  /** Waiting for the user's words: a draft's instruction or a custom rewrite. */
  asking: 'draft' | 'custom' | null
  replace: DocRange | null
  anchor: number
  /** The Markdown being rewritten, for a proofread's comparison. */
  source: string
  title: string
}

/** Why a request cannot go. */
type Blocked = 'image' | 'quote' | 'empty'

export type ComposeAiProps = {
  editor: Editor | null
  client: MailxClient
  draftId: string
  kind: DraftKind
  readOnly: boolean
  /** The toolbar's slot for the AI button. */
  toolbarSlot: HTMLElement | null
  /** Opens the assistant's island about this draft; the main window's inline composer only. */
  onAskAi?: () => void
}

/**
 * Writing help in the composer (ai-chat-ui.md 5.3): draft from an instruction, rewrite in a chosen
 * style, or proofread. The answer streams into a card right after the text it is about and changes
 * nothing until the user takes it: "采用" replaces the text in one undoable step, "插在下方" adds it
 * after. Editing the source meanwhile drops the card. A reply's quote, a forward's original and
 * inline images are never sent or changed.
 */
export function ComposeAi({ editor, client, draftId, kind, readOnly, toolbarSlot, onAskAi }: ComposeAiProps) {
  useTranslation('composer')
  const revision = useSettings(client).data?.revision ?? 0
  const availability = useQuery({
    queryKey: ['composeAssist', 'state', revision],
    queryFn: () => client.composeAssistState(),
    refetchOnWindowFocus: true,
    staleTime: 10_000,
  })
  const usable = availability.data?.usable === true && !readOnly && editor !== null
  const api = useMemo<RunApi>(() => ({
    start: (run) => run.type === 'composeAssist' ? client.composeAssistStart(run.draftId, run.task) : Promise.reject(new Error('not a compose run')),
    cancel: (runId) => client.composeAssistCancel(runId),
    snapshot: (runId) => client.composeAssistSnapshot(runId),
  }), [client])
  const run = useAssistantRun(client, { api })
  const [job, setJob] = useState<Job | null>(null)
  const [blocked, setBlocked] = useState<{ reason: Blocked; anchor: number } | null>(null)
  const [versions, setVersions] = useState<string[]>([])
  const [shown, setShown] = useState(0)
  const [consenting, setConsenting] = useState(false)
  const [menu, setMenu] = useState<'bubble' | 'toolbar' | null>(null)
  const [card] = useState(() => {
    const element = document.createElement('div')
    element.className = 'mail-ai-card-slot'
    element.contentEditable = 'false'
    return element
  })
  const [hint] = useState(() => {
    const element = document.createElement('span')
    element.className = 'mail-ai-hint'
    element.contentEditable = 'false'
    return element
  })
  const cardRef = useRef<HTMLDivElement>(null)

  const mark = (meta: ComposeAiMeta) => {
    if (!editor || editor.isDestroyed) return
    editor.view.dispatch(editor.state.tr.setMeta(composeAiKey, meta).setMeta('addToHistory', false))
  }

  // An edit inside the text being rewritten makes the answer meaningless. The plugin reports it from
  // inside an editor update; the card goes once that update is over.
  const dropStale = useRef(() => {})
  useEffect(() => {
    dropStale.current = () => {
      run.cancel()
      setJob(null)
      setVersions([])
      mark({ marks: null })
    }
  })
  useEffect(() => {
    if (!editor || editor.isDestroyed) return
    editor.registerPlugin(composeAiPlugin(card, hint, () => queueMicrotask(() => dropStale.current())))
    return () => { if (!editor.isDestroyed) editor.unregisterPlugin(composeAiKey) }
  }, [editor, card, hint])

  const view = useEditorState({
    editor,
    selector: ({ editor: current }) => {
      if (!current || current.isDestroyed) return null
      const { from, to, empty } = current.state.selection
      const end = ownEnd(current.state.doc, kind)
      return { from, to, empty, focused: current.isFocused, ownEmpty: ownRange(current.state.doc, kind) === null, end }
    },
  })

  // The "let AI draft" hint: an empty own part, the caret in it, nothing else going on.
  const hintAt = usable && view && view.focused && view.empty && view.ownEmpty && view.from <= view.end && !job && !blocked ? view.from : null
  useEffect(() => {
    if (!editor || editor.isDestroyed) return
    if ((composeAiKey.getState(editor.state)?.hint ?? null) !== hintAt) mark({ hint: hintAt })
  })


  // Keep each finished answer, so retries can be compared.
  const finishedText = run.state === 'completed' ? run.text.trim() : ''
  const [kept, setKept] = useState<string | null>(null)
  if (finishedText && finishedText !== kept) {
    setKept(finishedText)
    setVersions((list) => [...list, finishedText])
    setShown(versions.length)
  }

  const focusCard = () => requestAnimationFrame(() => cardRef.current?.focus({ preventScroll: true }))

  const show = (next: Job) => {
    setBlocked(null)
    setJob(next)
    setVersions([])
    setShown(0)
    setKept(null)
    mark({ marks: { source: next.replace && next.task?.type !== 'draft' ? next.replace : null, anchor: next.anchor } satisfies ComposeAiMarks, hint: null })
    focusCard()
  }

  const go = (next: Job) => {
    show(next)
    if (next.task) void run.start({ type: 'composeAssist', draftId, task: next.task })
  }

  /** The selection, else the whole own part; `null` with a reason when it cannot be sent. */
  const target = (): { range: DocRange } | { reason: Blocked; anchor: number } | null => {
    if (!editor || editor.isDestroyed) return null
    const { doc, selection } = editor.state
    const end = ownEnd(doc, kind)
    const range = selection.empty ? ownRange(doc, kind) : { from: selection.from, to: selection.to }
    const anchor = blockEnd(doc, selection.to)
    if (!range) return { reason: 'empty', anchor }
    if (range.to > end) return { reason: 'quote', anchor }
    if (hasImage(doc, range)) return { reason: 'image', anchor }
    return { range }
  }

  const rewrite = (style: RewriteStyle | null, proofread = false) => {
    setMenu(null)
    if (!editor) return
    const found = target()
    if (!found) return
    if ('reason' in found) {
      setJob(null)
      setBlocked(found)
      mark({ marks: { source: null, anchor: found.anchor }, hint: null })
      focusCard()
      return
    }
    const text = rangeMarkdown(editor, found.range)
    const anchor = blockEnd(editor.state.doc, found.range.to)
    if (!style && !proofread) {
      show({ task: null, asking: 'custom', replace: found.range, anchor, source: text, title: t('composer:ai.title.custom') })
      return
    }
    const task: ComposeTask = proofread ? { type: 'proofread', text } : { type: 'rewrite', text, style: style! }
    const title = proofread ? t('composer:ai.title.proofread')
      : style!.type === 'translate' ? t('composer:ai.title.translate', { language: nativeLanguageName(style!.code) })
        : style!.type === 'custom' ? t('composer:ai.title.custom') : t('composer:ai.title.rewrite', { style: styleLabel(style!.type as Style) })
    go({ task, asking: null, replace: found.range, anchor, source: text, title })
  }

  const draft = () => {
    setMenu(null)
    if (!editor) return
    const { doc, selection } = editor.state
    const end = ownEnd(doc, kind)
    // Inside the own part: the empty block at the caret is replaced, else the draft goes after its block.
    const at = Math.min(selection.from, Math.max(0, end - 1))
    const empty = emptyBlock(doc, at)
    const anchor = blockEnd(doc, at)
    show({ task: null, asking: 'draft', replace: empty, anchor, source: '', title: t('composer:ai.title.draft') })
  }

  const close = (refocus = true) => {
    run.cancel()
    setJob(null)
    setBlocked(null)
    setVersions([])
    mark({ marks: null })
    if (refocus && editor && !editor.isDestroyed) editor.view.focus()
  }

  const answer = versions[shown] ?? ''
  const take = (where: 'replace' | 'below') => {
    if (!editor || editor.isDestroyed || !job || !answer) return
    // Exactly what the preview showed: one function produced both.
    const html = answerHtml(editor, answer)
    const marks = composeAiKey.getState(editor.state)?.marks
    const anchor = marks?.anchor ?? job.anchor
    const source = marks?.source ?? job.replace
    const range = where === 'replace' && source ? source : { from: anchor, to: anchor }
    // One transaction: one step for Undo, and the card leaves with it.
    editor.chain().focus()
      .command(({ tr }) => { tr.setMeta(composeAiKey, { marks: null } satisfies ComposeAiMeta); return true })
      .insertContentAt(range, html)
      .run()
    run.cancel()
    setJob(null)
    setVersions([])
  }

  // Cmd/Ctrl+J in the body: the menu for a selection, a draft in an empty body, else the menu.
  useEffect(() => {
    if (!editor || editor.isDestroyed || !usable) return
    const dom = editor.view.dom
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.code !== 'KeyJ' || !(event.metaKey || event.ctrlKey) || event.altKey || event.shiftKey || event.isComposing) return
      event.preventDefault()
      // The main window's own Cmd/Ctrl+J (the island) must not see it.
      event.stopPropagation()
      const { selection, doc } = editor.state
      if (!selection.empty) setMenu('bubble')
      else if (!ownRange(doc, kind)) draft()
      else setMenu('toolbar')
    }
    dom.addEventListener('keydown', onKeyDown)
    return () => dom.removeEventListener('keydown', onKeyDown)
  })

  // The selection menu floats over the text; scrolling moves it with the text.
  const [, setScrolled] = useState(0)
  useEffect(() => {
    const onScroll = () => setScrolled((count) => count + 1)
    document.addEventListener('scroll', onScroll, true)
    return () => document.removeEventListener('scroll', onScroll, true)
  }, [])
  let bubble: { top: number; left: number } | null = null
  // A selection shows the menu; Cmd/Ctrl+J opens it even before React has seen the new selection.
  if (usable && view && (menu === 'bubble' || (view.focused && !view.empty)) && !job && !blocked && editor && !editor.isDestroyed) {
    try {
      const start = editor.view.coordsAtPos(view.from)
      bubble = { top: Math.max(4, start.top - 38), left: Math.max(4, start.left) }
    } catch {
      // No layout to measure (a hidden window): show it where the text begins rather than not at all.
      bubble = { top: 8, left: 8 }
    }
  }

  if (!usable) return null
  const rewriteItems = (
    <>
      {STYLES.map((style) => (
        <DropdownMenuItem key={style} onClick={() => rewrite({ type: style })}>{styleLabel(style)}</DropdownMenuItem>
      ))}
      <DropdownMenuSub>
        <DropdownMenuSubTrigger>{t('composer:ai.translateTo')}</DropdownMenuSubTrigger>
        <DropdownMenuSubContent className="max-h-72 w-44">
          {TRANSLATION_TARGETS.map((code) => (
            <DropdownMenuItem key={code} onClick={() => rewrite({ type: 'translate', code })}>{nativeLanguageName(code)}</DropdownMenuItem>
          ))}
        </DropdownMenuSubContent>
      </DropdownMenuSub>
      <DropdownMenuItem onClick={() => rewrite(null)}>{t('composer:ai.customItem')}</DropdownMenuItem>
    </>
  )

  return <>
    {toolbarSlot && createPortal(
      <DropdownMenu open={menu === 'toolbar'} onOpenChange={(open) => setMenu(open ? 'toolbar' : null)}>
        <Tooltip>
          <TooltipTrigger render={<DropdownMenuTrigger render={
            <Button variant="ghost" size="icon-sm" aria-label={t('composer:ai.button')} className="mail-format-button" onMouseDown={(event) => event.preventDefault()} />
          } />}>
            <AiMark className="size-4" />
          </TooltipTrigger>
          <TooltipContent>{t('composer:ai.buttonTip', { mod: MOD_KEY })}</TooltipContent>
        </Tooltip>
        <DropdownMenuContent className="w-48">
          <DropdownMenuItem onClick={draft}>{t('composer:ai.draftItem')}</DropdownMenuItem>
          <DropdownMenuSeparator />
          {rewriteItems}
          <DropdownMenuSeparator />
          <DropdownMenuItem onClick={() => rewrite(null, true)}>{t('composer:ai.proofread')}</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>,
      toolbarSlot,
    )}
    {bubble && createPortal(
      <div role="toolbar" aria-label={t('composer:ai.selectionTools')} className="mail-ai-bubble" style={{ top: bubble.top, left: bubble.left }}
        onMouseDown={(event) => event.preventDefault()}>
        <DropdownMenu open={menu === 'bubble'} onOpenChange={(open) => setMenu(open ? 'bubble' : null)}>
          <DropdownMenuTrigger render={<Button variant="ghost" size="sm" className="h-7 gap-1 px-2" />}>
            <AiMark /> {t('composer:ai.rewrite')} <HugeiconsIcon icon={ArrowDown01Icon} className="size-3" />
          </DropdownMenuTrigger>
          <DropdownMenuContent className="w-44">{rewriteItems}</DropdownMenuContent>
        </DropdownMenu>
        <Button variant="ghost" size="sm" className="h-7 px-2" onClick={() => rewrite(null, true)}>{t('composer:ai.proofread')}</Button>
      </div>,
      document.body,
    )}
    {hintAt !== null && createPortal(
      <button type="button" className="mail-ai-hint-button" onMouseDown={(event) => event.preventDefault()} onClick={draft}>
        <AiMark className="size-3" /> {t('composer:ai.hint', { mod: MOD_KEY })}
      </button>,
      hint,
    )}
    {(job || blocked) && createPortal(
      <ComposeAiCard
        cardRef={cardRef}
        job={job}
        blocked={blocked?.reason ?? null}
        run={run}
        answer={answer}
        versions={versions.length}
        shown={shown}
        editor={editor}
        onShow={setShown}
        onSubmit={(words, language) => {
          if (!job) return
          const task: ComposeTask = job.asking === 'draft'
            ? { type: 'draft', instruction: words, ...(language ? { language } : {}) }
            : { type: 'rewrite', text: job.source, style: { type: 'custom', instruction: words } }
          go({ ...job, task, asking: null })
        }}
        onRetry={() => { if (job?.task) void run.start({ type: 'composeAssist', draftId, task: job.task }) }}
        onTake={take}
        onClose={() => close()}
        onConsent={() => setConsenting(true)}
        onAskAi={onAskAi}
        onOpenLink={(url) => { void client.openExternalLink(url).catch(() => undefined) }}
        service={availability.data?.service ?? ''}
      />,
      card,
    )}
    <AiConsentDialog client={client} service={availability.data?.service ?? ''} open={consenting} onOpenChange={setConsenting}
      grant={() => client.composeAssistGrantConsent()}
      onGranted={() => { void availability.refetch(); if (job?.task) void run.start({ type: 'composeAssist', draftId, task: job.task }) }} />
  </>
}

type CardProps = {
  cardRef: React.RefObject<HTMLDivElement | null>
  job: Job | null
  blocked: Blocked | null
  run: ReturnType<typeof useAssistantRun>
  answer: string
  versions: number
  shown: number
  editor: Editor | null
  onShow: (index: number) => void
  onSubmit: (words: string, language: string | null) => void
  onRetry: () => void
  onTake: (where: 'replace' | 'below') => void
  onClose: () => void
  onConsent: () => void
  onAskAi?: () => void
  /** Opens a link the answer carries, as the editor does. */
  onOpenLink: (url: string) => void
  /** The AI connection's name, for the consent line. */
  service: string
}

function ComposeAiCard({ cardRef, job, blocked, run, answer, versions, shown, editor, onShow, onSubmit, onRetry, onTake, onClose, onConsent, onAskAi, onOpenLink, service }: CardProps) {
  useTranslation('composer')
  const reduced = usePrefersReducedMotion()
  const visible = useDocumentVisible()
  const [words, setWords] = useState('')
  const [language, setLanguage] = useState(AUTO)
  const composing = useRef(false)
  const working = run.state === 'starting' || run.state === 'running'
  const asking = job?.asking ?? null
  const done = !asking && !working && versions > 0 && answer !== ''
  const failed: ErrorCode | null = !asking && run.state === 'failed' ? run.error ?? 'internal' : null
  const consent = !asking && run.state === 'awaitingConsent'
  const stopped = !asking && !done && run.state === 'canceled'
  const proofread = job?.task?.type === 'proofread'
  const cut = done && run.finishedReason === 'length' && shown === versions - 1
  const parts = useMemo(() => {
    if (!done || !proofread || !editor || !job) return null
    return diff(answerText(editor, job.source), answerText(editor, answer))
  }, [done, proofread, editor, job, answer])
  const changes = parts?.filter((part) => part.kind !== 'same').length ?? 0

  const onKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (event.key === 'Escape') {
      event.preventDefault()
      event.stopPropagation()
      if (working) run.cancel()
      else onClose()
    } else if (event.key === 'Enter' && done && event.target === event.currentTarget) {
      event.preventDefault()
      onTake('replace')
    }
  }
  const languages: Record<string, string> = {
    [AUTO]: t('composer:ai.languageAuto'),
    ...Object.fromEntries(TRANSLATION_TARGETS.map((code) => [code, nativeLanguageName(code)])),
  }
  const blockedText = blocked === 'image' ? t('composer:ai.blockedImage') : blocked === 'quote' ? t('composer:ai.blockedQuote') : t('composer:ai.blockedEmpty')

  let body: React.ReactNode
  if (blocked) body = <p className="text-muted-foreground">{blockedText}</p>
  else if (asking) {
    body = (
      <form className="flex flex-col gap-2" onSubmit={(event) => { event.preventDefault(); if (!composing.current && words.trim()) onSubmit(words.trim(), asking === 'draft' && language !== AUTO ? language : null) }}>
        <textarea
          autoFocus
          rows={2}
          value={words}
          onChange={(event) => setWords(event.target.value)}
          onCompositionStart={() => { composing.current = true }}
          onCompositionEnd={() => { composing.current = false }}
          onKeyDown={(event) => {
            if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing && event.keyCode !== 229) {
              event.preventDefault()
              event.currentTarget.form?.requestSubmit()
            }
          }}
          aria-label={asking === 'draft' ? t('composer:ai.draftPrompt') : t('composer:ai.customPrompt')}
          placeholder={asking === 'draft' ? t('composer:ai.draftPlaceholder') : t('composer:ai.customPlaceholder')}
          maxLength={2000}
          className="w-full resize-none bg-transparent text-sm leading-relaxed outline-none placeholder:text-muted-foreground"
        />
        <div className="flex items-center gap-2">
          {asking === 'draft' && (
            <Select value={language} items={languages} onValueChange={(value: string | null) => value && setLanguage(value)}>
              <SelectTrigger size="sm" className="h-7 w-32 text-xs" aria-label={t('composer:ai.language')}><SelectValue /></SelectTrigger>
              <SelectContent className="max-h-72">
                <SelectItem value={AUTO}>{languages[AUTO]}</SelectItem>
                <SelectSeparator />
                {TRANSLATION_TARGETS.map((code) => <SelectItem key={code} value={code}>{languages[code]}</SelectItem>)}
              </SelectContent>
            </Select>
          )}
          <span className="flex-1" />
          <Button type="button" variant="ghost" size="sm" className="h-7" onClick={onClose}>{t('composer:ai.cancel')}</Button>
          <Button type="submit" size="sm" className="h-7" disabled={!words.trim()}>{t('composer:ai.generate')}</Button>
        </div>
      </form>
    )
  } else if (consent) body = <p>{t('composer:ai.consent', { service })}</p>
  else if (failed) body = <p>{t('composer:ai.failed')} · {errorText({ code: failed })}</p>
  else if (stopped) body = <p className="text-muted-foreground">{t('composer:ai.stopped')}</p>
  else if (working && !run.text.trim()) body = <p className="text-muted-foreground">{t('composer:ai.working')}</p>
  else if (working) body = <AnswerPreview editor={editor} text={run.text} onOpenLink={onOpenLink} />
  else if (done && parts) {
    body = changes === 0
      ? <p className="text-muted-foreground">{t('composer:ai.noChanges')}</p>
      : <p className="whitespace-pre-wrap" aria-label={t('composer:ai.changes', { count: changes })}>
        {parts.map((part, index) => part.kind === 'same' ? <span key={index}>{part.text}</span>
          : part.kind === 'removed' ? <del key={index} className="text-muted-foreground line-through decoration-destructive/70">{part.text}</del>
            : <ins key={index} className="rounded-sm bg-(--ai-2)/15 no-underline">{part.text}</ins>)}
      </p>
  } else if (done) body = <AnswerPreview editor={editor} text={answer} onOpenLink={onOpenLink} />

  return (
    <div ref={cardRef} tabIndex={-1} role="region" aria-label={job?.title ?? t('composer:ai.button')} aria-busy={working || undefined} onKeyDown={onKeyDown}
      className="mail-ai-card group/ai-card relative isolate my-2 rounded-lg text-sm outline-none select-text">
      <div aria-hidden className={cn('ai-glow absolute -inset-0.5 -z-10 rounded-[inherit] opacity-0 transition-opacity', reduced ? 'duration-100' : 'duration-150', working && 'opacity-[0.28]')} />
      <div className="relative rounded-[inherit] bg-background px-3 py-2 dark:bg-card">
        <div className="flex h-6 items-center gap-1.5 text-xs">
          <AiMark />
          <span className="font-medium">{job?.title ?? t('composer:ai.button')}</span>
          {proofread && done && changes > 0 && <span className="text-muted-foreground">· {t('composer:ai.changes', { count: changes })}</span>}
          <span className="flex-1" />
          {versions > 1 && !working && (
            <span className="flex items-center gap-0.5 text-muted-foreground tabular-nums">
              <Button variant="ghost" size="icon-xs" aria-label={t('composer:ai.previous')} disabled={shown === 0} onClick={() => onShow(shown - 1)}><HugeiconsIcon icon={ArrowLeft01Icon} /></Button>
              {shown + 1}/{versions}
              <Button variant="ghost" size="icon-xs" aria-label={t('composer:ai.next')} disabled={shown === versions - 1} onClick={() => onShow(shown + 1)}><HugeiconsIcon icon={ArrowRight01Icon} /></Button>
            </span>
          )}
        </div>
        <div className="mt-1 max-h-80 overflow-y-auto leading-relaxed">{body}</div>
        {cut && <p className="mt-1 text-xs text-muted-foreground">{t('composer:ai.cut')}</p>}
        {!asking && (
          <div className="mt-2 flex flex-wrap items-center gap-1.5">
            {done && (changes > 0 || !proofread) && <Button size="sm" className="h-7" onClick={() => onTake('replace')}>{job?.replace ? t('composer:ai.accept') : t('composer:ai.insert')} <kbd className="ml-1 text-[10px] opacity-70">↵</kbd></Button>}
            {done && !proofread && job?.replace && <Button size="sm" variant="outline" className="h-7" onClick={() => onTake('below')}>{t('composer:ai.insertBelow')}</Button>}
            {consent && <Button size="sm" className="h-7" onClick={onConsent}>{t('composer:ai.allow')}</Button>}
            {(done || failed || stopped) && <Button size="sm" variant="outline" className="h-7" onClick={onRetry}>{done ? t('composer:ai.again') : t('composer:ai.retry')}</Button>}
            {working && <Button size="sm" variant="outline" className="h-7" onClick={run.cancel}>{t('composer:ai.stop')} <kbd className="ml-1 text-[10px] opacity-70">Esc</kbd></Button>}
            {!working && <Button size="sm" variant="ghost" className="h-7" onClick={onClose}>{blocked ? t('composer:ai.dismiss') : t('composer:ai.discard')} <kbd className="ml-1 text-[10px] opacity-70">Esc</kbd></Button>}
            {done && onAskAi && <Button size="sm" variant="ghost" className="h-7 text-muted-foreground" onClick={onAskAi}>{t('composer:ai.ask')}</Button>}
          </div>
        )}
      </div>
      <div aria-hidden data-slot="ai-border" data-flow={working && !reduced && visible} className="ai-border pointer-events-none absolute inset-0 rounded-[inherit]" />
    </div>
  )
}

/** Renders the answer as the draft would hold it: the same HTML the card takes, in the editor's styles. */
function AnswerPreview({ editor, text, onOpenLink }: { editor: Editor | null; text: string; onOpenLink: (url: string) => void }) {
  const html = useMemo(() => editor && !editor.isDestroyed ? answerHtml(editor, text) : '', [editor, text])
  return <div
    className="mail-body-content mail-ai-preview"
    onClick={(event) => {
      const anchor = event.target instanceof Element ? event.target.closest('a') : null
      if (!anchor) return
      event.preventDefault()
      const href = anchor.getAttribute('href') ?? ''
      if (allowedMailLink(href)) onOpenLink(href)
    }}
    dangerouslySetInnerHTML={{ __html: html }}
  />
}
