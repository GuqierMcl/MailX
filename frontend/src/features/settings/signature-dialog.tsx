import { useEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { EditorContent, useEditor, useEditorState } from '@tiptap/react'
import { HugeiconsIcon } from '@hugeicons/react'
import {
  Image01Icon, Link01Icon, RedoIcon, TextAlignCenterIcon, TextAlignLeftIcon,
  TextAlignRightIcon, TextBoldIcon, TextClearIcon, TextColorIcon, TextItalicIcon,
  TextStrikethroughIcon, TextUnderlineIcon, UndoIcon,
} from '@hugeicons/core-free-icons'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import { ErrorNotice } from '@/components/error-notice'
import { IconButton } from '@/components/icon-button'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { Field, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import type { DraftAttachmentSummary, Signature, StagedSignatureImage } from '@/generated/accounts'
import { InlineImageController } from '@/features/composer/inline-image'
import { allowedMailLink, mailEditorOptions } from '@/features/composer/mail-body-editor-schema'
import { MAIL_COLORS, type MailColor, type MailTextAlign } from '@/features/composer/mail-body-format'
import '@/features/composer/mail-body-editor.css'
import type { MailxClient } from '@/lib/mailx-client'
import { useSaveSignature } from './signature-queries'

type SignatureDialogProps = {
  client: MailxClient
  editing: Signature | null
  onClose: () => void
}

const ALIGN_ITEMS: { value: MailTextAlign; readonly label: string; icon: typeof TextAlignLeftIcon }[] = [
  { value: 'left', get label() { return t('composer:toolbar.alignLeft') }, icon: TextAlignLeftIcon },
  { value: 'center', get label() { return t('composer:toolbar.alignCenter') }, icon: TextAlignCenterIcon },
  { value: 'right', get label() { return t('composer:toolbar.alignRight') }, icon: TextAlignRightIcon },
]

/** Focused modal for creating or editing a single signature with a compact single-row toolbar. */
export function SignatureDialog({ client, editing, onClose }: SignatureDialogProps) {
  useTranslation(['settings', 'composer'])
  const save = useSaveSignature(client)
  const [name, setName] = useState(editing?.name ?? '')
  const [bodyHtml, setBodyHtml] = useState(editing?.bodyHtml ?? '')
  const [stagedImages, setStagedImages] = useState<StagedSignatureImage[]>([])
  const [problem, setProblem] = useState<string | null>(null)
  const [linkOpen, setLinkOpen] = useState(false)
  const [linkHref, setLinkHref] = useState('')
  const [linkInvalid, setLinkInvalid] = useState(false)
  const [colorOpen, setColorOpen] = useState(false)
  const [alignOpen, setAlignOpen] = useState(false)
  const [pickingImage, setPickingImage] = useState(false)

  const signatureId = editing?.id ?? null
  const [images] = useState(() => new InlineImageController())

  const attachmentSummaries: DraftAttachmentSummary[] = useMemo(() => {
    const existing = (editing?.images ?? []).map((img) => ({
      id: img.id,
      fileName: img.fileName,
      mediaType: img.mediaType,
      size: img.size,
      origin: 'user' as const,
      inline: true,
      contentId: img.contentId,
    }))
    const staged = stagedImages.map((img) => ({
      id: img.id,
      fileName: img.fileName,
      mediaType: img.mediaType,
      size: img.size,
      origin: 'user' as const,
      inline: true,
      contentId: img.contentId,
    }))
    return [...existing, ...staged]
  }, [editing?.images, stagedImages])

  images.setLoader((imageId) => client.signatureImagePreview(signatureId, imageId))
  images.update(attachmentSummaries)
  useEffect(() => {
    images.attach()
    return () => images.detach()
  }, [images])

  const pasteRef = useRef<(files: File[]) => void>(() => undefined)
  const editor = useEditor({
    ...mailEditorOptions(images),
    content: editing?.bodyHtml || '<p></p>',
    editorProps: {
      attributes: {
        role: 'textbox',
        'aria-multiline': 'true',
        'aria-label': t('settings:signaturesBodyLabel'),
        class: 'mail-body-content min-h-[220px] max-h-[320px] overflow-y-auto px-3.5 py-3 text-sm leading-relaxed outline-none',
      },
      handlePaste(_view, event) {
        const files = Array.from(event.clipboardData?.files ?? []).filter((f) => f.type.startsWith('image/'))
        if (files.length === 0) return false
        event.preventDefault()
        pasteRef.current(files)
        return true
      },
    },
    onUpdate({ editor: current }) {
      setBodyHtml(current.getHTML())
    },
  })

  const insertStaged = (items: StagedSignatureImage[]) => {
    if (!editor || items.length === 0) return
    setStagedImages((prev) => [...prev, ...items])
    const nodes = items.map((img) => ({
      type: 'inlineImage',
      attrs: { src: `cid:${img.contentId}`, alt: img.fileName },
    }))
    editor.chain().focus().insertContent(nodes).run()
  }

  useEffect(() => {
    pasteRef.current = (files: File[]) => {
      setProblem(null)
      void (async () => {
        const added: StagedSignatureImage[] = []
        try {
          for (const file of files) {
            const bytes = new Uint8Array(await file.arrayBuffer())
            const staged = await client.addSignatureImageBytes(file.name || 'image.png', bytes)
            added.push(staged)
          }
          insertStaged(added)
        } catch (error) {
          setProblem(errorText(error))
        }
      })()
    }
  })

  const state = useEditorState({
    editor,
    selector: ({ editor: e }) => ({
      bold: e?.isActive('bold') ?? false,
      italic: e?.isActive('italic') ?? false,
      underline: e?.isActive('underline') ?? false,
      strike: e?.isActive('strike') ?? false,
      link: e?.isActive('link') ?? false,
      color: (e?.getAttributes('textStyle')?.mailColor as MailColor | undefined) ?? null,
      align: ((e?.getAttributes('paragraph')?.textAlign ?? 'left') as MailTextAlign),
      canUndo: e?.can().undo() ?? false,
      canRedo: e?.can().redo() ?? false,
    }),
  })

  const pickImages = async () => {
    if (pickingImage) return
    setPickingImage(true)
    setProblem(null)
    try {
      const picked = await client.addSignatureImages()
      if (picked && picked.length > 0) insertStaged(picked)
    } catch (error) {
      setProblem(errorText(error))
    } finally {
      setPickingImage(false)
    }
  }

  const openLinkPopover = () => {
    if (!editor) return
    const existing = (editor.getAttributes('link')?.href as string | undefined) ?? ''
    setLinkHref(existing)
    setLinkInvalid(false)
    setLinkOpen(true)
  }

  const applyLink = () => {
    if (!editor) return
    const trimmed = linkHref.trim()
    if (!trimmed) {
      editor.chain().focus().unsetLink().run()
      setLinkOpen(false)
      return
    }
    const candidate = /^(https?:\/\/|mailto:)/i.test(trimmed) ? trimmed : `https://${trimmed}`
    if (!allowedMailLink(candidate)) {
      setLinkInvalid(true)
      return
    }
    editor.chain().focus().extendMarkRange('link').setLink({ href: candidate }).run()
    setLinkOpen(false)
  }

  const submit = async () => {
    const trimmedName = name.trim()
    if (!trimmedName) {
      setProblem(t('settings:signaturesNameRequired'))
      return
    }
    setProblem(null)
    try {
      await save.mutateAsync({
        id: signatureId,
        name: trimmedName,
        bodyHtml: editor ? editor.getHTML() : bodyHtml,
        stagedImages,
      })
      onClose()
    } catch (error) {
      setProblem(errorText(error))
    }
  }

  const activeAlignIcon = ALIGN_ITEMS.find((item) => item.value === state?.align)?.icon ?? TextAlignLeftIcon

  return (
    <Dialog open onOpenChange={(open) => !open && !save.isPending && onClose()}>
      <DialogContent className="sm:max-w-[540px]">
        <DialogHeader>
          <DialogTitle>{editing ? t('settings:signaturesDialogEdit') : t('settings:signaturesDialogNew')}</DialogTitle>
          <DialogDescription className="sr-only">{t('settings:signaturesDialogDescription')}</DialogDescription>
        </DialogHeader>
        <div className="flex flex-col gap-4">
          <Field>
            <FieldLabel htmlFor="signature.name">{t('settings:signaturesNameLabel')}</FieldLabel>
            <Input
              id="signature.name"
              value={name}
              onChange={(event) => { setName(event.target.value); if (problem) setProblem(null) }}
              placeholder={t('settings:signaturesNamePlaceholder')}
              maxLength={120}
              autoFocus
            />
          </Field>
          <Field>
            <FieldLabel>{t('settings:signaturesBodyLabel')}</FieldLabel>
            <div className="overflow-hidden rounded-lg border border-input bg-background focus-within:ring-2 focus-within:ring-ring/30">
              <div role="toolbar" aria-label={t('composer:toolbar.format')} className="flex flex-wrap items-center gap-0.5 border-b border-border/60 bg-muted/30 px-1.5 py-1">
                <IconButton className="mail-format-button" label={t('composer:toolbar.bold')} aria-pressed={state?.bold} onClick={() => editor?.chain().focus().toggleBold().run()}>
                  <HugeiconsIcon icon={TextBoldIcon} aria-hidden />
                </IconButton>
                <IconButton className="mail-format-button" label={t('composer:toolbar.italic')} aria-pressed={state?.italic} onClick={() => editor?.chain().focus().toggleItalic().run()}>
                  <HugeiconsIcon icon={TextItalicIcon} aria-hidden />
                </IconButton>
                <IconButton className="mail-format-button" label={t('composer:toolbar.underline')} aria-pressed={state?.underline} onClick={() => editor?.chain().focus().toggleUnderline().run()}>
                  <HugeiconsIcon icon={TextUnderlineIcon} aria-hidden />
                </IconButton>
                <IconButton className="mail-format-button" label={t('composer:toolbar.strike')} aria-pressed={state?.strike} onClick={() => editor?.chain().focus().toggleStrike().run()}>
                  <HugeiconsIcon icon={TextStrikethroughIcon} aria-hidden />
                </IconButton>
                <span className="mx-1" />
                <Popover open={colorOpen} onOpenChange={setColorOpen}>
                  <PopoverTrigger render={
                    <IconButton className="mail-format-button" label={t('composer:toolbar.textColor')} aria-pressed={Boolean(state?.color)}>
                      <HugeiconsIcon icon={TextColorIcon} aria-hidden />
                    </IconButton>
                  } />
                  <PopoverContent align="start" className="w-auto p-2">
                    <div className="flex items-center gap-1">
                      <button
                        type="button"
                        aria-label={t('composer:toolbar.auto')}
                        onClick={() => { editor?.chain().focus().unsetMailColor().run(); setColorOpen(false) }}
                        className="flex size-6 items-center justify-center rounded border border-border text-[10px] hover:bg-muted"
                      >
                        A
                      </button>
                      {MAIL_COLORS.map((color) => (
                        <button
                          key={color.name}
                          type="button"
                          aria-label={color.label}
                          data-color={color.name}
                          onClick={() => { editor?.chain().focus().setMailColor(color.name).run(); setColorOpen(false) }}
                          className="mail-color-swatch size-6 rounded-full border border-foreground/15 transition-transform hover:scale-110"
                        />
                      ))}
                    </div>
                  </PopoverContent>
                </Popover>
                <Popover open={alignOpen} onOpenChange={setAlignOpen}>
                  <PopoverTrigger render={
                    <IconButton className="mail-format-button" label={t('composer:toolbar.alignLeft')} aria-pressed={state?.align !== 'left'}>
                      <HugeiconsIcon icon={activeAlignIcon} aria-hidden />
                    </IconButton>
                  } />
                  <PopoverContent align="start" className="w-auto p-1">
                    <div className="flex items-center gap-0.5">
                      {ALIGN_ITEMS.map((item) => (
                        <IconButton
                          key={item.value}
                          className="mail-format-button"
                          label={item.label}
                          aria-pressed={state?.align === item.value}
                          onClick={() => { editor?.chain().focus().setMailTextAlign(item.value).run(); setAlignOpen(false) }}
                        >
                          <HugeiconsIcon icon={item.icon} aria-hidden />
                        </IconButton>
                      ))}
                    </div>
                  </PopoverContent>
                </Popover>
                <span className="mx-1" />
                <Popover open={linkOpen} onOpenChange={setLinkOpen}>
                  <PopoverTrigger render={
                    <IconButton className="mail-format-button" label={t('composer:toolbar.link')} aria-pressed={state?.link} onClick={openLinkPopover}>
                      <HugeiconsIcon icon={Link01Icon} aria-hidden />
                    </IconButton>
                  } />
                  <PopoverContent align="start" className="w-72 p-2.5">
                    <div className="flex flex-col gap-2">
                      <Input
                        value={linkHref}
                        onChange={(event) => { setLinkHref(event.target.value); setLinkInvalid(false) }}
                        onKeyDown={(event) => { if (event.key === 'Enter') { event.preventDefault(); applyLink() } }}
                        placeholder="https://example.com"
                        aria-label={t('composer:toolbar.linkAddress')}
                        autoFocus
                      />
                      {linkInvalid && <p className="text-xs text-destructive">{t('composer:toolbar.invalidLink')}</p>}
                      <div className="flex justify-end gap-1.5">
                        <Button type="button" variant="ghost" size="xs" onClick={() => setLinkOpen(false)}>{t('composer:action.cancel')}</Button>
                        <Button type="button" size="xs" onClick={applyLink}>{t('composer:action.apply')}</Button>
                      </div>
                    </div>
                  </PopoverContent>
                </Popover>
                <IconButton label={t('composer:toolbar.insertImage')} disabled={pickingImage} onClick={() => void pickImages()}>
                  <HugeiconsIcon icon={Image01Icon} aria-hidden />
                </IconButton>
                <IconButton
                  label={t('composer:toolbar.clearFormatting')}
                  onClick={() => editor?.chain().focus().unsetAllMarks().unsetMailTextAlign().run()}
                >
                  <HugeiconsIcon icon={TextClearIcon} aria-hidden />
                </IconButton>
                <span className="ml-auto flex items-center gap-0.5">
                  <IconButton label={t('composer:action.undo')} disabled={!state?.canUndo} onClick={() => editor?.chain().focus().undo().run()}>
                    <HugeiconsIcon icon={UndoIcon} aria-hidden />
                  </IconButton>
                  <IconButton label={t('composer:toolbar.redo')} disabled={!state?.canRedo} onClick={() => editor?.chain().focus().redo().run()}>
                    <HugeiconsIcon icon={RedoIcon} aria-hidden />
                  </IconButton>
                </span>
              </div>
              <EditorContent editor={editor} />
            </div>
          </Field>
          {problem && <ErrorNotice message={problem} />}
        </div>
        <DialogFooter>
          <Button type="button" variant="ghost" disabled={save.isPending} onClick={onClose}>
            {t('settings:signaturesCancel')}
          </Button>
          <Button type="button" disabled={save.isPending} onClick={() => void submit()}>
            {save.isPending ? t('settings:signaturesSaving') : t('settings:signaturesSave')}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
