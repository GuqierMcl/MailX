import { useEffect, useImperativeHandle, useRef, useState, type Ref } from 'react'
import { EditorContent, useEditor, useEditorState } from '@tiptap/react'
import { DOMParser as ProseMirrorDOMParser } from '@tiptap/pm/model'
import { EditorState, Selection } from '@tiptap/pm/state'
import type { Editor } from '@tiptap/core'
import { HugeiconsIcon } from '@hugeicons/react'
import {
  LeftToRightListBulletIcon, LeftToRightListNumberIcon, Link01Icon, MoreHorizontalIcon,
  QuoteDownIcon, RedoIcon, TextBoldIcon, TextClearIcon, TextItalicIcon,
  TextStrikethroughIcon, TextUnderlineIcon, UndoIcon,
} from '@hugeicons/core-free-icons'
import { IconButton } from '@/components/icon-button'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { allowedMailLink, mailEditorOptions } from './mail-body-editor-schema'
import './mail-body-editor.css'

export type MailBodySelection = { from: number; to: number }
export type MailBodyEditorHandle = {
  focus: (position?: 'start' | 'end') => void
  getSelection: () => MailBodySelection | null
  setSelection: (selection: MailBodySelection) => void
}

type MailBodyEditorProps = {
  /** HTML returned by the core, which has already applied the outgoing whitelist. */
  value: string
  onChange: (html: string) => void
  readOnly?: boolean
  editorRef?: Ref<MailBodyEditorHandle>
}

type LinkPopup = MailBodySelection & { href: string }
type FormatAction = {
  label: string
  shortcut?: string
  icon: typeof TextBoldIcon
  mark?: string
  execute: (editor: Editor) => void
}

const markActions: FormatAction[] = [
  { label: '粗体', shortcut: 'B', icon: TextBoldIcon, mark: 'bold', execute: (editor) => editor.commands.toggleBold() },
  { label: '斜体', shortcut: 'I', icon: TextItalicIcon, mark: 'italic', execute: (editor) => editor.commands.toggleItalic() },
  { label: '下划线', shortcut: 'U', icon: TextUnderlineIcon, mark: 'underline', execute: (editor) => editor.commands.toggleUnderline() },
  { label: '删除线', shortcut: 'Shift+S', icon: TextStrikethroughIcon, mark: 'strike', execute: (editor) => editor.commands.toggleStrike() },
]
const blockActions: FormatAction[] = [
  { label: '无序列表', shortcut: 'Shift+8', icon: LeftToRightListBulletIcon, mark: 'bulletList', execute: (editor) => editor.commands.toggleBulletList() },
  { label: '有序列表', shortcut: 'Shift+7', icon: LeftToRightListNumberIcon, mark: 'orderedList', execute: (editor) => editor.commands.toggleOrderedList() },
  { label: '引用', shortcut: 'Shift+B', icon: QuoteDownIcon, mark: 'blockquote', execute: (editor) => editor.commands.toggleBlockquote() },
  { label: '清除格式', icon: TextClearIcon, execute: (editor) => editor.chain().unsetAllMarks().clearNodes().run() },
]

/** A host-independent editor. Draft revisions, saving and sending live outside. */
export default function MailBodyEditor({ value, onChange, readOnly = false, editorRef }: MailBodyEditorProps) {
  const [linkPopup, setLinkPopup] = useState<LinkPopup | null>(null)
  const [linkError, setLinkError] = useState(false)
  const [moreOpen, setMoreOpen] = useState(false)
  const root = useRef<HTMLDivElement>(null)
  const linkInput = useRef<HTMLInputElement>(null)
  const popup = useRef<HTMLDivElement>(null)
  const moreButton = useRef<HTMLButtonElement>(null)
  const linkComposing = useRef(false)
  const options = mailEditorOptions()
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
      if (transaction.docChanged) { setLinkPopup(null); setMoreOpen(false) }
    },
    editorProps: {
      ...options.editorProps,
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

  const hasPopup = (!readOnly && linkPopup !== null) || moreOpen
  useEffect(() => {
    if (!hasPopup) return
    const outside = (event: PointerEvent) => {
      if (event.target instanceof Node && !popup.current?.contains(event.target) && !moreButton.current?.contains(event.target)) {
        setLinkPopup(null)
        setMoreOpen(false)
      }
    }
    document.addEventListener('pointerdown', outside)
    return () => document.removeEventListener('pointerdown', outside)
  }, [hasPopup])

  useEffect(() => {
    if (linkPopup && !readOnly) linkInput.current?.focus()
    else if (moreOpen) popup.current?.querySelector<HTMLButtonElement>('button:not(:disabled)')?.focus()
  }, [linkPopup, moreOpen, readOnly])

  function openLink() {
    if (!editor || !editor.isEditable) return
    editor.commands.extendMarkRange('link')
    const { from, to } = editor.state.selection
    linkComposing.current = false
    setLinkError(false)
    setMoreOpen(false)
    setLinkPopup({ from, to, href: String(editor.getAttributes('link').href ?? '') })
  }

  function closePopup() {
    linkComposing.current = false
    setLinkPopup(null)
    setMoreOpen(false)
    if (editor && !editor.isDestroyed) editor.view.focus()
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
      <IconButton label="撤销" tip="撤销（Cmd/Ctrl+Z）" size="icon-sm" disabled={readOnly || !state?.undo} role={inMenu ? 'menuitem' : undefined}
        onMouseDown={(event) => event.preventDefault()} onClick={() => { if (editor?.isEditable) { editor.view.focus(); editor.commands.undo() }; setMoreOpen(false) }}>
        <HugeiconsIcon icon={UndoIcon} strokeWidth={1.8} aria-hidden="true" />
      </IconButton>
      <IconButton label="重做" tip="重做（Cmd/Ctrl+Shift+Z）" size="icon-sm" disabled={readOnly || !state?.redo} role={inMenu ? 'menuitem' : undefined}
        onMouseDown={(event) => event.preventDefault()} onClick={() => { if (editor?.isEditable) { editor.view.focus(); editor.commands.redo() }; setMoreOpen(false) }}>
        <HugeiconsIcon icon={RedoIcon} strokeWidth={1.8} aria-hidden="true" />
      </IconButton>
    </>
  )

  return (
    <div ref={root} className="mail-body-editor">
      <div role="toolbar" aria-label="正文格式" className="mail-format-toolbar">
        <div className="mail-format-group">{markActions.map((action) => actionButton(action))}</div>
        <div className="mail-format-group">
          <IconButton label="链接" tip="链接（Cmd/Ctrl+K）" size="icon-sm" disabled={readOnly || !editor} aria-pressed={state?.link ?? false}
            className="mail-format-button" onMouseDown={(event) => event.preventDefault()} onClick={openLink}>
            <HugeiconsIcon icon={Link01Icon} strokeWidth={1.8} aria-hidden="true" />
          </IconButton>
        </div>
        <div className="mail-format-group mail-format-secondary">{blockActions.slice(0, 2).map((action) => actionButton(action))}</div>
        <div className="mail-format-group mail-format-secondary">{actionButton(blockActions[2])}</div>
        <div className="mail-format-group mail-format-secondary">{actionButton(blockActions[3])}</div>
        <div className="mail-format-group mail-format-secondary">{historyButtons()}</div>
        <div className="mail-format-more">
          <IconButton ref={moreButton} label="更多格式" size="icon-sm" disabled={readOnly || !editor} aria-haspopup="menu" aria-expanded={moreOpen}
            onMouseDown={(event) => event.preventDefault()} onClick={() => { setLinkPopup(null); setMoreOpen(!moreOpen) }}>
            <HugeiconsIcon icon={MoreHorizontalIcon} strokeWidth={1.8} aria-hidden="true" />
          </IconButton>
        </div>
      </div>
      {!readOnly && linkPopup && (
        <div ref={popup} role="dialog" aria-label="编辑链接" className="mail-editor-popup mail-link-popup"
          onKeyDownCapture={(event) => {
            if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); closePopup() }
            if (event.key === 'Enter' && (event.ctrlKey || event.metaKey || event.nativeEvent.isComposing || event.keyCode === 229 || linkComposing.current)) {
              event.preventDefault()
              event.stopPropagation()
            }
          }}>
          <form onSubmit={(event) => { event.preventDefault(); if (!linkComposing.current) applyLink() }}>
            <label className="text-xs font-medium" htmlFor="mail-link-address">链接地址</label>
            <Input ref={linkInput} id="mail-link-address" defaultValue={linkPopup.href} placeholder="https://example.com" aria-invalid={linkError} aria-describedby={linkError ? 'mail-link-error' : undefined}
              onChange={() => setLinkError(false)} onCompositionStart={() => { linkComposing.current = true }} onCompositionEnd={() => { linkComposing.current = false }} autoComplete="off" />
            {linkError && <p id="mail-link-error" role="alert" className="text-xs text-destructive">请输入 http、https 或 mailto 地址。</p>}
            <div className="flex justify-end gap-2">
              {linkPopup.href && <Button type="button" variant="ghost" size="sm" onClick={() => applyLink(true)}>移除链接</Button>}
              <Button type="button" variant="ghost" size="sm" onClick={closePopup}>取消</Button>
              <Button type="submit" size="sm">应用</Button>
            </div>
          </form>
        </div>
      )}
      {!readOnly && moreOpen && (
        <div ref={popup} role="menu" aria-label="更多正文格式" className="mail-editor-popup mail-format-menu"
          onKeyDownCapture={(event) => {
            if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); closePopup(); return }
            const buttons = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>('button:not(:disabled)'))
            const index = buttons.indexOf(document.activeElement as HTMLButtonElement)
            let next: number
            if (event.key === 'ArrowRight' || event.key === 'ArrowDown') next = (index + 1) % buttons.length
            else if (event.key === 'ArrowLeft' || event.key === 'ArrowUp') next = (index - 1 + buttons.length) % buttons.length
            else if (event.key === 'Home') next = 0
            else if (event.key === 'End') next = buttons.length - 1
            else return
            event.preventDefault()
            buttons[next]?.focus()
          }}>
          {blockActions.map((action) => actionButton(action, true))}{historyButtons(true)}
        </div>
      )}
      <EditorContent editor={editor} className="mail-body-surface" />
    </div>
  )
}
