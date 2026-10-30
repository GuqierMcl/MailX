import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import { useEffect, useImperativeHandle, useRef, useState, type Ref, type KeyboardEvent as ReactKeyboardEvent } from 'react'
import { EditorContent, useEditor, useEditorState } from '@tiptap/react'
import { DOMParser as ProseMirrorDOMParser } from '@tiptap/pm/model'
import { EditorState, Selection } from '@tiptap/pm/state'
import type { Editor } from '@tiptap/core'
import { HugeiconsIcon } from '@hugeicons/react'
import {
  LeftToRightListBulletIcon, LeftToRightListNumberIcon, Link01Icon, MoreHorizontalIcon,
  QuoteDownIcon, RedoIcon, TextBoldIcon, TextClearIcon, TextItalicIcon,
  TextStrikethroughIcon, TextUnderlineIcon, UndoIcon, ArrowDown01Icon,
  Image01Icon, TextAlignLeftIcon, TextAlignCenterIcon, TextAlignRightIcon, TextColorIcon, PaintBoardIcon, Tick02Icon,
} from '@hugeicons/core-free-icons'
import { IconButton } from '@/components/icon-button'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import type { DraftAttachmentSummary, InlineImage } from '@/generated/accounts'
import { InlineImageController } from './inline-image'
import { allowedMailLink, mailEditorOptions } from './mail-body-editor-schema'
import { MAIL_COLORS, MAIL_HIGHLIGHTS, type MailColor, type MailHighlight, type MailTextAlign } from './mail-body-format'
import './mail-body-editor.css'

export type MailBodySelection = { from: number; to: number }
export type MailBodyEditorHandle = {
  focus: (position?: 'start' | 'end') => void
  getSelection: () => MailBodySelection | null
  setSelection: (selection: MailBodySelection) => void
  /** Inserts the images at the cursor, in order, and leaves the cursor after the last one. */
  insertInlineImages: (images: readonly InlineImage[]) => void
}

type MailBodyEditorProps = {
  /** HTML returned by the core, which has already applied the outgoing whitelist. */
  value: string
  onChange: (html: string) => void
  readOnly?: boolean
  editorRef?: Ref<MailBodyEditorHandle>
  /** The draft's attachments, to resolve `cid:` references, and the host call that renders one. */
  inlineImages?: { attachments: readonly DraftAttachmentSummary[]; loadPreview: (attachmentId: string) => Promise<ArrayBuffer> }
  /** Shows the image button when set. */
  onInsertImage?: () => void
  /** Image files found on the clipboard; the host stores them and inserts the nodes. */
  onPasteImages?: (files: File[]) => void
}

type LinkPopup = MailBodySelection & { href: string }
type FormatPopup = MailBodySelection & { kind: 'paragraph' | 'color' | 'highlight' }

const alignActions = [
  { value: 'left', get label() { return t('composer:toolbar.alignLeft') }, icon: TextAlignLeftIcon },
  { value: 'center', get label() { return t('composer:toolbar.alignCenter') }, icon: TextAlignCenterIcon },
  { value: 'right', get label() { return t('composer:toolbar.alignRight') }, icon: TextAlignRightIcon },
] as const

function popupButtons(container: HTMLElement) {
  return Array.from(container.querySelectorAll<HTMLButtonElement>('button:not(:disabled)')).filter((button) => {
    for (let element: HTMLElement | null = button; element && element !== container; element = element.parentElement) {
      if (getComputedStyle(element).display === 'none') return false
    }
    return true
  })
}
type FormatAction = {
  label: string
  shortcut?: string
  icon: typeof TextBoldIcon
  mark?: string
  execute: (editor: Editor) => void
}

const markActions: FormatAction[] = [
  { get label() { return t('composer:toolbar.bold') }, shortcut: 'B', icon: TextBoldIcon, mark: 'bold', execute: (editor) => editor.commands.toggleBold() },
  { get label() { return t('composer:toolbar.italic') }, shortcut: 'I', icon: TextItalicIcon, mark: 'italic', execute: (editor) => editor.commands.toggleItalic() },
  { get label() { return t('composer:toolbar.underline') }, shortcut: 'U', icon: TextUnderlineIcon, mark: 'underline', execute: (editor) => editor.commands.toggleUnderline() },
  { get label() { return t('composer:toolbar.strike') }, shortcut: 'Shift+S', icon: TextStrikethroughIcon, mark: 'strike', execute: (editor) => editor.commands.toggleStrike() },
]
const blockActions: FormatAction[] = [
  { get label() { return t('composer:toolbar.bulletList') }, shortcut: 'Shift+8', icon: LeftToRightListBulletIcon, mark: 'bulletList', execute: (editor) => editor.commands.toggleBulletList() },
  { get label() { return t('composer:toolbar.orderedList') }, shortcut: 'Shift+7', icon: LeftToRightListNumberIcon, mark: 'orderedList', execute: (editor) => editor.commands.toggleOrderedList() },
  { get label() { return t('composer:toolbar.quote') }, shortcut: 'Shift+B', icon: QuoteDownIcon, mark: 'blockquote', execute: (editor) => editor.commands.toggleBlockquote() },
  { get label() { return t('composer:toolbar.clearFormatting') }, icon: TextClearIcon, execute: (editor) => editor.chain().unsetAllMarks().command(({ tr }) => {
    // unsetAllMarks only affects a range; an empty caret needs explicit clearing.
    if (tr.selection.empty) tr.setStoredMarks([])
    return true
  }).unsetMailTextAlign().run() },
]

/** A host-independent editor. Draft revisions, saving and sending live outside. */
export default function MailBodyEditor({ value, onChange, readOnly = false, editorRef, inlineImages, onInsertImage, onPasteImages }: MailBodyEditorProps) {
  const { i18n } = useTranslation('composer')
  const [linkPopup, setLinkPopup] = useState<LinkPopup | null>(null)
  const [linkError, setLinkError] = useState(false)
  const [moreOpen, setMoreOpen] = useState(false)
  const [formatPopup, setFormatPopup] = useState<FormatPopup | null>(null)
  const root = useRef<HTMLDivElement>(null)
  const linkInput = useRef<HTMLInputElement>(null)
  const popup = useRef<HTMLDivElement>(null)
  const moreButton = useRef<HTMLButtonElement>(null)
  const linkComposing = useRef(false)
  const pasteImages = useRef(onPasteImages)
  useEffect(() => { pasteImages.current = onPasteImages }, [onPasteImages])
  const [images] = useState(() => new InlineImageController())
  if (inlineImages) images.setLoader(inlineImages.loadPreview)
  // Synchronous so the first paint already knows which images exist.
  images.update(inlineImages?.attachments ?? [])
  useEffect(() => {
    images.attach()
    return () => images.detach()
  }, [images])
  const options = mailEditorOptions(images)
  const editor = useEditor({
    ...options,
    content: value,
    editable: !readOnly,
    autofocus: false,
    // Tiptap's hook updates callbacks without recreating the editor.
    onUpdate: ({ editor: current }) => onChange(current.getHTML()),
    onTransaction: ({ transaction }) => {
      // A conflict reload can replace the document while a link dialog is open.
      // Its saved range belongs to the old document and must not be applied.
      if (transaction.docChanged) { setLinkPopup(null); setMoreOpen(false); setFormatPopup(null) }
    },
    editorProps: {
      ...options.editorProps,
      handlePaste(view, event) {
        const files = Array.from(event.clipboardData?.files ?? []).filter((file) => file.type.startsWith('image/'))
        if (!view.editable || files.length === 0 || !pasteImages.current) return false
        event.preventDefault()
        pasteImages.current(files)
        return true
      },
      handleKeyDown(view, event) {
        if (event.isComposing || !view.editable) return false
        if ((event.metaKey || event.ctrlKey) && !event.altKey && event.key.toLowerCase() === 'k') {
          event.preventDefault()
          openLink()
          return true
        }
        return false
      },
      handleDOMEvents: {
        click(view, event) {
          const anchor = event.target instanceof Element ? event.target.closest('a') : null
          if (!anchor || !view.dom.contains(anchor)) return false
          // This also blocks navigation for locked (read-only) drafts.
          event.preventDefault()
          if (view.editable && editor) {
            const position = view.posAtDOM(anchor, 0)
            editor.commands.setTextSelection(position)
            openLink()
          }
          return true
        },
        auxclick(view, event) {
          const anchor = event.target instanceof Element ? event.target.closest('a') : null
          if (!anchor || !view.dom.contains(anchor)) return false
          event.preventDefault()
          return true
        },
      },
    },
  })
  const state = useEditorState({
    editor,
    selector: ({ editor: current }) => current && !current.isDestroyed ? {
      active: [...markActions, ...blockActions].filter((action) => action.mark && current.isActive(action.mark)).map((action) => action.mark),
      link: current.isActive('link'),
      heading: ([1, 2, 3] as const).find((level) => current.isActive('heading', { level })) ?? 0,
      align: current.isActive({ textAlign: 'center' }) ? 'center' : current.isActive({ textAlign: 'right' }) ? 'right' : 'left',
      color: String(current.getAttributes('mailColor').color ?? ''),
      highlight: String(current.getAttributes('mailHighlight').highlight ?? ''),
      undo: current.can().undo(),
      redo: current.can().redo(),
    } : null,
  })

  useImperativeHandle(editorRef, () => ({
    focus: (position) => {
      if (!editor || editor.isDestroyed) return
      if (position) editor.commands.setTextSelection((position === 'start' ? Selection.atStart(editor.state.doc) : Selection.atEnd(editor.state.doc)).from)
      editor.view.focus()
    },
    getSelection: () => editor && !editor.isDestroyed ? { from: editor.state.selection.from, to: editor.state.selection.to } : null,
    setSelection: (selection) => { if (editor && !editor.isDestroyed) editor.commands.setTextSelection(selection) },
    insertInlineImages: (inserted) => {
      if (!editor || editor.isDestroyed || !editor.isEditable || inserted.length === 0) return
      editor.chain().focus().insertContent(inserted.map((image) => ({ type: 'inlineImage', attrs: { cid: image.contentId, alt: image.fileName } }))).run()
    },
  }), [editor])

  useEffect(() => {
    // EditorContent is briefly detached by StrictMode/Suspense. Focus only the
    // final mounted view, and cancel retired instances' pending work. Unlike the
    // built-in autofocus this never steals focus from a popup opened meanwhile.
    const frame = requestAnimationFrame(() => {
      if (editor && !editor.isDestroyed && editor.isEditable && editor.view.dom.isConnected && !popup.current) editor.view.focus()
    })
    return () => cancelAnimationFrame(frame)
  }, [editor])

  useEffect(() => {
    if (!editor || editor.isDestroyed) return
    editor.setEditable(!readOnly, false)
    editor.view.dom.setAttribute('aria-readonly', String(readOnly))
  }, [editor, readOnly])

  useEffect(() => {
    if (editor && !editor.isDestroyed) editor.view.dom.setAttribute('aria-label', t('composer:field.body'))
  }, [editor, i18n.resolvedLanguage])

  useEffect(() => {
    if (!editor || editor.isDestroyed || editor.getHTML() === value) return
    const template = document.createElement('template')
    template.innerHTML = value
    const incoming = ProseMirrorDOMParser.fromSchema(editor.schema).parse(template.content, { preserveWhitespace: 'full' })
    // The core may canonicalize equivalent tags (b/strong, i/em). A successful
    // autosave must not move the cursor, break IME input, or erase undo history.
    if (!incoming.eq(editor.state.doc)) {
      editor.commands.setContent(incoming.toJSON(), { emitUpdate: false })
      // Reinitializing the plugins clears history as well as stored marks. A
      // conflict reload must not let Undo resurrect discarded local changes.
      editor.view.updateState(EditorState.create({
        doc: incoming,
        plugins: editor.state.plugins,
        selection: Selection.atStart(incoming),
      }))
      // Let toolbar subscribers observe the reset, without producing an edit.
      editor.view.dispatch(editor.state.tr.setMeta('addToHistory', false))
    }
  }, [editor, value])

  const hasPopup = !readOnly && (linkPopup !== null || formatPopup !== null || moreOpen)
  useEffect(() => {
    if (!hasPopup) return
    const outside = (event: PointerEvent) => {
      if (event.target instanceof Node && !popup.current?.contains(event.target) && !root.current?.querySelector('[role=toolbar]')?.contains(event.target)) {
        setLinkPopup(null)
        setMoreOpen(false)
        setFormatPopup(null)
      }
    }
    document.addEventListener('pointerdown', outside)
    return () => document.removeEventListener('pointerdown', outside)
  }, [hasPopup])

  useEffect(() => {
    if (linkPopup && !readOnly) linkInput.current?.focus()
    else if (!readOnly && popup.current && (moreOpen || formatPopup)) {
      const buttons = popupButtons(popup.current)
      ;(buttons.find((button) => button.getAttribute('aria-checked') === 'true') ?? buttons[0])?.focus()
    }
  }, [linkPopup, moreOpen, formatPopup, readOnly])

  function openLink() {
    if (!editor || !editor.isEditable) return
    editor.commands.extendMarkRange('link')
    const { from, to } = editor.state.selection
    linkComposing.current = false
    setLinkError(false)
    setMoreOpen(false)
    setFormatPopup(null)
    setLinkPopup({ from, to, href: String(editor.getAttributes('link').href ?? '') })
  }

  function closePopup() {
    linkComposing.current = false
    setLinkPopup(null)
    setMoreOpen(false)
    setFormatPopup(null)
    if (editor && !editor.isDestroyed) editor.view.focus()
  }

  function openFormat(kind: FormatPopup['kind']) {
    if (!editor?.isEditable) return
    const { from, to } = editor.state.selection
    setLinkPopup(null)
    setMoreOpen(false)
    setFormatPopup(formatPopup?.kind === kind ? null : { kind, from, to })
  }

  function applyFormat(value: string) {
    if (!editor?.isEditable || !formatPopup) return
    // Re-selecting the same collapsed caret would clear pending bold/color marks.
    if (editor.state.selection.from !== formatPopup.from || editor.state.selection.to !== formatPopup.to) {
      editor.commands.setTextSelection({ from: formatPopup.from, to: formatPopup.to })
    }
    if (formatPopup.kind === 'paragraph') {
      const pendingMarks = editor.state.selection.empty ? editor.state.storedMarks : null
      if (value === '0') editor.commands.setParagraph()
      else editor.commands.setHeading({ level: Number(value) as 1 | 2 | 3 })
      if (pendingMarks) editor.view.dispatch(editor.state.tr.setStoredMarks(pendingMarks))
    } else if (formatPopup.kind === 'color') {
      if (value) editor.commands.setMailColor(value as MailColor)
      else editor.commands.unsetMailColor()
    } else if (value) editor.commands.setMailHighlight(value as MailHighlight)
    else editor.commands.unsetMailHighlight()
    closePopup()
  }

  function menuKeyDown(event: ReactKeyboardEvent<HTMLDivElement>, columns = 1) {
    if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); closePopup(); return }
    if (event.key === 'Enter' && (event.ctrlKey || event.metaKey || event.nativeEvent.isComposing || event.keyCode === 229)) {
      event.preventDefault(); event.stopPropagation(); return
    }
    const buttons = popupButtons(event.currentTarget)
    const index = buttons.indexOf(document.activeElement as HTMLButtonElement)
    let next: number
    if (event.key === 'ArrowRight') next = index + 1
    else if (event.key === 'ArrowDown') next = index + columns
    else if (event.key === 'ArrowLeft') next = index - 1
    else if (event.key === 'ArrowUp') next = index - columns
    else if (event.key === 'Home') next = 0
    else if (event.key === 'End') next = buttons.length - 1
    else return
    event.preventDefault()
    event.stopPropagation()
    buttons[(next + buttons.length) % buttons.length]?.focus()
  }

  function applyLink(remove = false) {
    if (!editor || !linkPopup || !editor.isEditable) return
    const href = linkInput.current?.value.trim() ?? ''
    if (!remove && !allowedMailLink(href)) {
      setLinkError(true)
      return
    }
    const chain = editor.chain().setTextSelection({ from: linkPopup.from, to: linkPopup.to })
    if (remove) chain.unsetLink().run()
    else if (linkPopup.from === linkPopup.to) {
      chain.insertContent({ type: 'text', text: href, marks: [{ type: 'link', attrs: { href } }] }).unsetLink().run()
    } else chain.setLink({ href }).run()
    closePopup()
  }

  const actionButton = (action: FormatAction, inMenu = false) => (
    <IconButton
      key={action.label}
      label={action.label}
      tip={action.shortcut ? `${action.label}（Cmd/Ctrl+${action.shortcut}）` : action.label}
      size="icon-sm"
      disabled={readOnly || !editor}
      aria-pressed={action.mark ? state?.active.includes(action.mark) ?? false : undefined}
      className="mail-format-button"
      role={inMenu ? 'menuitem' : undefined}
      onMouseDown={(event) => event.preventDefault()}
      onClick={() => {
        if (editor?.isEditable) { editor.view.focus(); action.execute(editor) }
        if (inMenu) setMoreOpen(false)
      }}
    ><HugeiconsIcon icon={action.icon} strokeWidth={1.8} aria-hidden="true" /></IconButton>
  )

  const historyButtons = (inMenu = false) => (
    <>
      <IconButton label={t('composer:action.undo')} tip={t('composer:toolbar.undoShortcut')} size="icon-sm" disabled={readOnly || !state?.undo} role={inMenu ? 'menuitem' : undefined}
        onMouseDown={(event) => event.preventDefault()} onClick={() => { if (editor?.isEditable) { editor.view.focus(); editor.commands.undo() }; setMoreOpen(false) }}>
        <HugeiconsIcon icon={UndoIcon} strokeWidth={1.8} aria-hidden="true" />
      </IconButton>
      <IconButton label={t('composer:toolbar.redo')} tip={t('composer:toolbar.redoShortcut')} size="icon-sm" disabled={readOnly || !state?.redo} role={inMenu ? 'menuitem' : undefined}
        onMouseDown={(event) => event.preventDefault()} onClick={() => { if (editor?.isEditable) { editor.view.focus(); editor.commands.redo() }; setMoreOpen(false) }}>
        <HugeiconsIcon icon={RedoIcon} strokeWidth={1.8} aria-hidden="true" />
      </IconButton>
    </>
  )

  const alignButtons = (inMenu = false) => alignActions.map((action) => (
    <IconButton key={action.value} label={action.label} size="icon-sm" disabled={readOnly || !editor}
      role={inMenu ? 'menuitem' : undefined} aria-pressed={state?.align === action.value} className="mail-format-button"
      onMouseDown={(event) => event.preventDefault()} onClick={() => {
        if (editor?.isEditable) editor.commands.setMailTextAlign(action.value as MailTextAlign)
        closePopup()
      }}><HugeiconsIcon icon={action.icon} strokeWidth={1.8} aria-hidden="true" /></IconButton>
  ))

  const colorButtons = (inMenu = false) => (
    <>
      <IconButton label={t('composer:toolbar.textColor')} size="icon-sm" disabled={readOnly || !editor} role={inMenu ? 'menuitem' : undefined}
        aria-haspopup="menu" aria-expanded={formatPopup?.kind === 'color'} className="mail-format-button"
        onMouseDown={(event) => event.preventDefault()} onClick={() => openFormat('color')}>
        <span className="mail-format-color-indicator" data-color={state?.color || undefined}><HugeiconsIcon icon={TextColorIcon} strokeWidth={1.8} aria-hidden="true" /></span>
      </IconButton>
      <IconButton label={t('composer:toolbar.highlight')} size="icon-sm" disabled={readOnly || !editor} role={inMenu ? 'menuitem' : undefined}
        aria-haspopup="menu" aria-expanded={formatPopup?.kind === 'highlight'} className="mail-format-button"
        onMouseDown={(event) => event.preventDefault()} onClick={() => openFormat('highlight')}>
        <span className="mail-format-color-indicator" data-highlight={state?.highlight || undefined}><HugeiconsIcon icon={PaintBoardIcon} strokeWidth={1.8} aria-hidden="true" /></span>
      </IconButton>
    </>
  )
  const formatChoices = formatPopup?.kind === 'paragraph'
    ? [{ name: '0', get label() { return t('composer:field.body') } }, ...[1, 2, 3].map((level) => ({ name: String(level), label: t('composer:toolbar.heading', { level }) }))]
    : [{ name: '', label: formatPopup?.kind === 'color' ? t('composer:toolbar.auto') : t('composer:toolbar.none') }, ...(formatPopup?.kind === 'color' ? MAIL_COLORS : MAIL_HIGHLIGHTS)]
  const formatValue = formatPopup?.kind === 'paragraph' ? String(state?.heading ?? 0) : formatPopup?.kind === 'color' ? state?.color : state?.highlight

  return (
    <div ref={root} className="mail-body-editor">
      <div role="toolbar" aria-label={t('composer:toolbar.format')} className="mail-format-toolbar">
        <div className="mail-format-group">
          <IconButton label={t('composer:toolbar.paragraphCurrent', { style: state?.heading ? t('composer:toolbar.heading', { level: state.heading }) : t('composer:field.body') })} tip={t('composer:toolbar.paragraphShortcut')} size="icon-sm"
            disabled={readOnly || !editor} aria-haspopup="menu" aria-expanded={formatPopup?.kind === 'paragraph'} className="mail-paragraph-trigger"
            onMouseDown={(event) => event.preventDefault()} onClick={() => openFormat('paragraph')}>
            <span>{state?.heading ? t('composer:toolbar.heading', { level: state.heading }) : t('composer:field.body')}</span><HugeiconsIcon icon={ArrowDown01Icon} strokeWidth={1.8} aria-hidden="true" />
          </IconButton>
        </div>
        <div className="mail-format-group">{markActions.map((action) => actionButton(action))}</div>
        <div className="mail-format-group mail-format-colors">{colorButtons()}</div>
        <div className="mail-format-group">
          <IconButton label={t('composer:toolbar.link')} tip={t('composer:toolbar.linkShortcut')} size="icon-sm" disabled={readOnly || !editor} aria-pressed={state?.link ?? false}
            className="mail-format-button" onMouseDown={(event) => event.preventDefault()} onClick={openLink}>
            <HugeiconsIcon icon={Link01Icon} strokeWidth={1.8} aria-hidden="true" />
          </IconButton>
          {onInsertImage && (
            <IconButton label={t('composer:toolbar.insertImage')} size="icon-sm" disabled={readOnly || !editor}
              className="mail-format-button" onMouseDown={(event) => event.preventDefault()} onClick={onInsertImage}>
              <HugeiconsIcon icon={Image01Icon} strokeWidth={1.8} aria-hidden="true" />
            </IconButton>
          )}
        </div>
        <div className="mail-format-group mail-format-secondary">{alignButtons()}</div>
        <div className="mail-format-group">{blockActions.slice(0, 2).map((action) => actionButton(action))}</div>
        <div className="mail-format-group mail-format-secondary">{actionButton(blockActions[2])}</div>
        <div className="mail-format-group">{actionButton(blockActions[3])}</div>
        <div className="mail-format-group mail-format-secondary">{historyButtons()}</div>
        <div className="mail-format-more">
          <IconButton ref={moreButton} label={t('composer:toolbar.more')} size="icon-sm" disabled={readOnly || !editor} aria-haspopup="menu" aria-expanded={moreOpen}
            onMouseDown={(event) => event.preventDefault()} onClick={() => { setLinkPopup(null); setFormatPopup(null); setMoreOpen(!moreOpen) }}>
            <HugeiconsIcon icon={MoreHorizontalIcon} strokeWidth={1.8} aria-hidden="true" />
          </IconButton>
        </div>
      </div>
      {!readOnly && linkPopup && (
        <div ref={popup} role="dialog" aria-label={t('composer:toolbar.editLink')} className="mail-editor-popup mail-link-popup"
          onKeyDownCapture={(event) => {
            if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); closePopup() }
            if (event.key === 'Enter' && (event.ctrlKey || event.metaKey || event.nativeEvent.isComposing || event.keyCode === 229 || linkComposing.current)) {
              event.preventDefault()
              event.stopPropagation()
            }
          }}>
          <form onSubmit={(event) => { event.preventDefault(); if (!linkComposing.current) applyLink() }}>
            <label className="text-xs font-medium" htmlFor="mail-link-address">{t('composer:toolbar.linkAddress')}</label>
            <Input ref={linkInput} id="mail-link-address" defaultValue={linkPopup.href} placeholder="https://example.com" aria-invalid={linkError} aria-describedby={linkError ? 'mail-link-error' : undefined}
              onChange={() => setLinkError(false)} onCompositionStart={() => { linkComposing.current = true }} onCompositionEnd={() => { linkComposing.current = false }} autoComplete="off" />
            {linkError && <p id="mail-link-error" role="alert" className="text-xs text-destructive">{t('composer:toolbar.invalidLink')}</p>}
            <div className="flex justify-end gap-2">
              {linkPopup.href && <Button type="button" variant="ghost" size="sm" onClick={() => applyLink(true)}>{t('composer:toolbar.removeLink')}</Button>}
              <Button type="button" variant="ghost" size="sm" onClick={closePopup}>{t('composer:action.cancel')}</Button>
              <Button type="submit" size="sm">{t('composer:action.apply')}</Button>
            </div>
          </form>
        </div>
      )}
      {!readOnly && moreOpen && (
        <div ref={popup} role="menu" aria-label={t('composer:toolbar.moreBody')} className="mail-editor-popup mail-format-menu"
          onKeyDownCapture={(event) => menuKeyDown(event)}>
          <div className="mail-format-group mail-menu-colors">{colorButtons(true)}</div>
          <div className="mail-format-group">{alignButtons(true)}</div>
          <div className="mail-format-group">{actionButton(blockActions[2], true)}</div>
          <div className="mail-format-group">{historyButtons(true)}</div>
        </div>
      )}
      {!readOnly && formatPopup && (
        <div ref={popup} role="menu" aria-label={formatPopup.kind === 'paragraph' ? t('composer:toolbar.paragraph') : formatPopup.kind === 'color' ? t('composer:toolbar.textColor') : t('composer:toolbar.highlightColor')}
          className={`mail-editor-popup mail-choice-popup ${formatPopup.kind === 'paragraph' ? 'mail-paragraph-menu' : 'mail-palette-menu'}`}
          onKeyDownCapture={(event) => menuKeyDown(event, formatPopup.kind === 'paragraph' ? 1 : 3)}>
          {formatChoices.map((choice) => (
            <IconButton key={choice.name} label={choice.label} tip={formatPopup.kind === 'paragraph' && choice.name !== '0' ? `${choice.label}（Cmd/Ctrl+Alt+${choice.name}）` : choice.label}
              role="menuitemradio" aria-checked={formatValue === choice.name} className="mail-choice-button" size="sm"
              onMouseDown={(event) => event.preventDefault()} onClick={() => applyFormat(choice.name)}>
              {formatPopup.kind !== 'paragraph' && <span aria-hidden="true" className="mail-color-swatch" data-swatch-color={formatPopup.kind === 'color' ? choice.name : undefined} data-swatch-highlight={formatPopup.kind === 'highlight' ? choice.name : undefined} />}
              <span>{choice.label}</span>
              {formatValue === choice.name && <HugeiconsIcon icon={Tick02Icon} strokeWidth={1.8} aria-hidden="true" />}
            </IconButton>
          ))}
        </div>
      )}
      <EditorContent editor={editor} className="mail-body-surface" />
    </div>
  )
}
