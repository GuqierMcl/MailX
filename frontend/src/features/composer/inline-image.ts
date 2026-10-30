import { t } from '@/i18n'
import type { DraftAttachmentSummary } from '@/generated/accounts'
import { mergeAttributes, Node } from '@tiptap/core'

/** Previews stay in memory for as long as the editor is mounted; the draft's own bytes live in the core. */
type PreviewLoader = (attachmentId: string) => Promise<ArrayBuffer>

/**
 * Resolves `cid:` references in the body to displayable object URLs. The cache is keyed by attachment id, so the
 * same image shown twice (or re-rendered after an edit) is fetched once. `release` revokes every URL created.
 */
export class InlineImageController {
  private byContentId = new Map<string, string>()
  private cache = new Map<string, Promise<string | null>>()
  private urls = new Set<string>()
  private listeners = new Set<() => void>()
  private generation = 0
  private readonly onUnload = () => this.release()

  private loadPreview: PreviewLoader = () => Promise.reject(new Error('no preview loader'))

  setLoader(loadPreview: PreviewLoader) {
    this.loadPreview = loadPreview
  }

  /** Ties the cache to a mounted editor and to the window's lifetime. */
  attach() {
    window.addEventListener('pagehide', this.onUnload)
  }

  /** Called whenever the draft's attachment list changes. */
  update(attachments: readonly DraftAttachmentSummary[]) {
    const next = new Map<string, string>()
    for (const attachment of attachments) if (attachment.inline && attachment.contentId) next.set(attachment.contentId, attachment.id)
    const live = new Set(next.values())
    let changed = next.size !== this.byContentId.size
    for (const [contentId, id] of next) if (this.byContentId.get(contentId) !== id) changed = true
    this.byContentId = next
    // A removed attachment's preview is no longer reachable from the body.
    for (const [id, pending] of this.cache) {
      if (live.has(id)) continue
      this.cache.delete(id)
      void pending.then((url) => { if (url) this.revoke(url) })
    }
    if (changed) for (const listener of [...this.listeners]) listener()
  }

  has(contentId: string): boolean {
    return this.byContentId.has(contentId)
  }

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  /** An object URL for the image, or `null` when no inline attachment has that Content-ID or the preview is gone. */
  load(contentId: string): Promise<string | null> {
    const id = this.byContentId.get(contentId)
    if (!id) return Promise.resolve(null)
    const cached = this.cache.get(id)
    if (cached) return cached
    const generation = this.generation
    const pending = this.loadPreview(id).then(
      (bytes) => {
        const url = URL.createObjectURL(new Blob([bytes]))
        if (generation !== this.generation) { URL.revokeObjectURL(url); return null }
        this.urls.add(url)
        return url
      },
      (cause: unknown) => {
        // A missing attachment stays missing; other failures may succeed on the next render.
        if ((cause as { code?: unknown } | null)?.code !== 'notFound' && this.cache.get(id) === pending) this.cache.delete(id)
        return null
      },
    )
    this.cache.set(id, pending)
    return pending
  }

  /** Revokes every object URL. The controller stays usable; later loads fetch again. */
  release() {
    this.generation += 1
    for (const url of this.urls) URL.revokeObjectURL(url)
    this.urls.clear()
    this.cache.clear()
  }

  /** Unmount: revokes every URL and stops listening for window unload. */
  detach() {
    window.removeEventListener('pagehide', this.onUnload)
    this.release()
  }

  private revoke(url: string) {
    if (this.urls.delete(url)) URL.revokeObjectURL(url)
  }
}

function dimension(value: string | null): number | null {
  const parsed = value && /^\d{1,5}$/.test(value) ? Number(value) : 0
  return parsed > 0 ? parsed : null
}

export type InlineImageOptions = { controller: InlineImageController | null }

/**
 * An image the body references as `cid:<Content-ID>`. Any other `<img>` (remote URLs, data URIs) is not part of the
 * editing schema, so it never enters the editor; the core applies the same rule when saving.
 */
export const InlineImageNode = Node.create<InlineImageOptions>({
  name: 'inlineImage',
  group: 'inline',
  inline: true,
  atom: true,
  draggable: false,
  selectable: true,

  addOptions() {
    return { controller: null }
  },

  addAttributes() {
    return {
      cid: {
        default: '',
        parseHTML: (element) => (element.getAttribute('src') ?? '').slice('cid:'.length),
        renderHTML: (attributes) => ({ src: `cid:${String(attributes.cid)}` }),
      },
      alt: {
        default: null,
        parseHTML: (element) => element.getAttribute('alt'),
        renderHTML: (attributes) => (attributes.alt ? { alt: String(attributes.alt) } : {}),
      },
      width: {
        default: null,
        parseHTML: (element) => dimension(element.getAttribute('width')),
        renderHTML: (attributes) => (attributes.width ? { width: String(attributes.width) } : {}),
      },
      height: {
        default: null,
        parseHTML: (element) => dimension(element.getAttribute('height')),
        renderHTML: (attributes) => (attributes.height ? { height: String(attributes.height) } : {}),
      },
    }
  },

  parseHTML() {
    return [{ tag: 'img[src^="cid:"]' }]
  },

  renderHTML({ HTMLAttributes }) {
    return ['img', mergeAttributes(HTMLAttributes)]
  },

  addNodeView() {
    const controller = this.options.controller
    return ({ node }) => {
      let current = node
      let token = 0
      const dom = document.createElement('span')
      dom.className = 'mail-inline-image'
      dom.contentEditable = 'false'

      const showMissing = () => {
        dom.replaceChildren()
        const placeholder = document.createElement('span')
        placeholder.className = 'mail-inline-image-missing'
        placeholder.setAttribute('role', 'img')
        placeholder.textContent = t('composer:inlineImage.missing')
        placeholder.setAttribute('aria-label', t('composer:inlineImage.missing'))
        dom.append(placeholder)
      }
      const render = () => {
        const mine = ++token
        const cid = String(current.attrs.cid)
        if (!controller || !controller.has(cid)) { showMissing(); return }
        void controller.load(cid).then((url) => {
          if (mine !== token) return
          if (!url) { showMissing(); return }
          const image = document.createElement('img')
          image.src = url
          image.alt = String(current.attrs.alt ?? '')
          if (current.attrs.width) image.width = Number(current.attrs.width)
          if (current.attrs.height) image.height = Number(current.attrs.height)
          dom.replaceChildren(image)
        })
      }
      render()
      const unsubscribe = controller?.subscribe(render)

      return {
        dom,
        update(next) {
          if (next.type !== current.type) return false
          const changed = next.attrs.cid !== current.attrs.cid || next.attrs.alt !== current.attrs.alt || next.attrs.width !== current.attrs.width || next.attrs.height !== current.attrs.height
          current = next
          if (changed) render()
          return true
        },
        ignoreMutation: () => true,
        destroy() {
          token += 1
          unsubscribe?.()
        },
      }
    }
  },
})
