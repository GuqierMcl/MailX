import { invoke, isTauri } from '@tauri-apps/api/core'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import type { ApiResult } from '@/generated/accounts'

export type WindowPlatform = 'macos' | 'windows' | 'linux'
export type WindowButton = 'minimize' | 'maximize' | 'close'
export type WindowAction = 'minimize' | 'toggleMaximize' | 'close' | 'startDragging' | 'titleBarDoubleClick'

export type WindowState = { maximized: boolean; fullscreen: boolean; focused: boolean }

export type WindowChrome = {
  platform: WindowPlatform
  /** Controls drawn before the title content (GTK layouts may put them on the left). */
  leading: WindowButton[]
  /** Controls drawn after the title content. */
  trailing: WindowButton[]
  state: WindowState
}

/** The window behind the title bar: the desktop host, or a dev preview in the browser. */
export interface WindowHost {
  getChrome(): Promise<WindowChrome>
  perform(action: WindowAction): void
  onStateChange(listener: (state: WindowState) => void): () => void
}

const STATE_EVENT = 'mailx://window-state'

export function createDesktopWindowHost(): WindowHost | null {
  if (!isTauri()) return null
  return {
    async getChrome() {
      const result = await invoke<ApiResult<WindowChrome>>('window_get_chrome')
      if (result.ok === false) throw new Error(result.error.code)
      return result.data
    },
    perform(action) {
      // A failed window gesture has nothing useful to show; the window simply stays as it is.
      invoke('window_perform', { action }).catch(() => undefined)
    },
    onStateChange(listener) {
      let disposed = false
      let unlisten: (() => void) | undefined
      getCurrentWebviewWindow()
        .listen<WindowState>(STATE_EVENT, (event) => listener(event.payload))
        .then((stop) => (disposed ? stop() : (unlisten = stop)), () => undefined)
      return () => {
        disposed = true
        unlisten?.()
      }
    },
  }
}

const PREVIEW_BUTTONS: Record<WindowPlatform, WindowButton[]> = {
  macos: [],
  windows: ['minimize', 'maximize', 'close'],
  linux: ['minimize', 'maximize', 'close'],
}

/** Draws another platform's controls in the browser, e.g. `?titlebar=windows`. Gestures only flip local state. */
export function createPreviewWindowHost(platform: WindowPlatform): WindowHost {
  let state: WindowState = { maximized: false, fullscreen: false, focused: true }
  const listeners = new Set<(state: WindowState) => void>()
  return {
    getChrome: async () => ({ platform, leading: [], trailing: PREVIEW_BUTTONS[platform], state }),
    perform(action) {
      if (action !== 'toggleMaximize' && action !== 'titleBarDoubleClick') return
      state = { ...state, maximized: !state.maximized }
      listeners.forEach((listener) => listener(state))
    },
    onStateChange(listener) {
      listeners.add(listener)
      return () => listeners.delete(listener)
    },
  }
}

export function previewPlatformFromUrl(search: string): WindowPlatform | null {
  const platform = new URLSearchParams(search).get('titlebar')
  return platform === 'macos' || platform === 'windows' || platform === 'linux' ? platform : null
}
