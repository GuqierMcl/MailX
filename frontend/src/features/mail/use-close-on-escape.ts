import { useEffect } from 'react'

/** True for places where Esc already means something: text fields, menus, dialogs. */
function escapeBelongsToSomethingElse(event: KeyboardEvent) {
  if (event.defaultPrevented || event.isComposing) return true
  const target = event.target instanceof Element ? event.target : null
  if (target?.closest('input, textarea, select, [contenteditable=""], [contenteditable="true"]')) return true
  // An open dialog, menu or popup takes Esc for itself first.
  return Boolean(document.querySelector('[role="dialog"][data-open], [role="menu"], [role="listbox"], [role="alertdialog"][data-open]'))
}

/**
 * Closes the reader on Esc. It stays out of the way of anything that uses Esc
 * for its own purpose, so a search box, the command palette or a dialog is
 * dismissed first and the message stays open.
 */
export function useCloseOnEscape(enabled: boolean, onClose: () => void) {
  useEffect(() => {
    if (!enabled) return
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || escapeBelongsToSomethingElse(event)) return
      onClose()
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [enabled, onClose])
}
