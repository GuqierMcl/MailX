import { useEffect } from 'react'
import type { SettingsTarget } from './settings-dialog'

const EVENT = 'mailx:open-settings'

/** Asks the main window's workbench to open the settings dialog, from anywhere (the title bar). */
export function requestSettings(target: SettingsTarget) {
  window.dispatchEvent(new CustomEvent<SettingsTarget>(EVENT, { detail: target }))
}

export function useSettingsRequests(onRequest: (target: SettingsTarget) => void) {
  useEffect(() => {
    const listener = (event: Event) => onRequest((event as CustomEvent<SettingsTarget>).detail)
    window.addEventListener(EVENT, listener)
    return () => window.removeEventListener(EVENT, listener)
  }, [onRequest])
}
