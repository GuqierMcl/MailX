import type { EditCommand } from './model'

const textInputs = 'textarea, input:not([type]), input:is([type="text"], [type="search"], [type="url"], [type="tel"], [type="email"], [type="password"])'

export function editContext(element: Element | null) {
  const control = element instanceof HTMLElement && element.matches(textInputs)
    ? element as HTMLInputElement | HTMLTextAreaElement
    : null
  const target = control ?? (element instanceof HTMLElement && element.isContentEditable ? element : null)
  const editable = target !== null && !control?.readOnly && !control?.disabled
  const selected = control
    ? control.selectionStart !== control.selectionEnd && control.getAttribute('type') !== 'password'
    : Boolean(document.getSelection()?.toString())
  return { target, editable, selected }
}

export function canEdit(command: EditCommand, context: ReturnType<typeof editContext>) {
  if (command === 'copy') return context.selected
  if (command === 'cut') return context.editable && context.selected
  if (command === 'selectAll') return context.target !== null
  if (command === 'paste') return context.editable
  return context.editable && (context.target?.closest('.tiptap') !== null || document.queryCommandEnabled?.(command) === true)
}

export async function runEdit(command: EditCommand, element: Element | null) {
  if (element instanceof HTMLElement && element.isConnected) element.focus({ preventScroll: true })
  const context = editContext(element)
  if (!canEdit(command, context)) return
  if ((command === 'undo' || command === 'redo') && context.target?.closest('.tiptap')) {
    const mac = /Mac|iP(?:hone|od|ad)/.test(navigator.platform)
    context.target.dispatchEvent(new KeyboardEvent('keydown', {
      key: command === 'redo' ? 'Z' : 'z', code: 'KeyZ', ctrlKey: !mac, metaKey: mac,
      shiftKey: command === 'redo', bubbles: true, cancelable: true,
    }))
    return
  }
  if (document.execCommand?.(command)) return
  if (command === 'paste' && navigator.clipboard) {
    const text = await navigator.clipboard.readText()
    if (document.activeElement === context.target) document.execCommand?.('insertText', false, text)
  }
}
