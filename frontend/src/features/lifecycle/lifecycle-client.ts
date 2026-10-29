import { invoke, isTauri } from '@tauri-apps/api/core'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import type { ApiResult } from '@/generated/accounts'
import { clientFailure, MailxClientError, type MailxClient } from '@/lib/mailx-client'
import type { WindowPlatform } from '@/features/window-chrome/window-host'

export type LifecycleSnapshot = {
  revision: number
  platform: WindowPlatform
  backgroundAvailable: boolean
  closeRequestId: number | null
  quitRequestId: number | null
  quitting: boolean
  quitTimedOut: boolean
}
export type CloseAction = 'background' | 'quit' | 'cancel'
export type DesktopAction = 'settings' | 'compose'
export type CloseChoice = { requestId: number; action: CloseAction; remember: boolean }

/** Only the trusted main window owns application lifecycle controls. */
export interface LifecycleClient {
  getSnapshot(): Promise<LifecycleSnapshot>
  onSnapshot(listener: (snapshot: LifecycleSnapshot) => void): Promise<() => void>
  resolveClose(choice: CloseChoice): Promise<void>
  quit(): Promise<void>
  cancelQuit(generation?: number): Promise<void>
  confirmQuit(generation: number): Promise<void>
  onDesktopAction(listener: (action: DesktopAction) => void): Promise<() => void>
  setDesktopReady(ready: boolean): Promise<void>
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  let result: ApiResult<T>
  try { result = await invoke<ApiResult<T>>(command, args) } catch { throw new MailxClientError(clientFailure(null)) }
  if (!result.ok) throw new MailxClientError(result.error)
  return result.data
}

export function createDesktopLifecycleClient(): LifecycleClient | null {
  if (!isTauri()) return null
  return {
    getSnapshot: () => call<LifecycleSnapshot>('app_get_lifecycle'),
    onSnapshot: (listener) => getCurrentWebviewWindow().listen<LifecycleSnapshot>('mailx://lifecycle', ({ payload }) => listener(payload)),
    resolveClose: (choice) => call<void>('app_resolve_close', choice),
    quit: () => call<void>('app_quit'),
    cancelQuit: (generation) => call<void>('app_cancel_quit', { generation: generation ?? null }),
    confirmQuit: (generation) => call<void>('app_confirm_quit', { generation }),
    onDesktopAction: (listener) => getCurrentWebviewWindow().listen<DesktopAction>('mailx://desktop-action', ({ payload }) => listener(payload)),
    setDesktopReady: (ready) => call<void>('app_desktop_ready', { ready }),
  }
}

/** Browser previews simulate a close without closing the browser tab. */
export function createPreviewLifecycleClient(platform: WindowPlatform, settings: Pick<MailxClient, 'getSettings' | 'updateSettings'>, backgroundAvailable = true) {
  let snapshot: LifecycleSnapshot = { revision: 0, platform, backgroundAvailable, closeRequestId: null, quitRequestId: null, quitting: false, quitTimedOut: false }
  let requestId = 0
  const listeners = new Set<(snapshot: LifecycleSnapshot) => void>()
  const publish = (patch: Partial<LifecycleSnapshot>) => {
    snapshot = { ...snapshot, ...patch, revision: snapshot.revision + 1 }
    listeners.forEach((listener) => listener(snapshot))
  }
  const client: LifecycleClient & { requestClose(): Promise<void> } = {
    getSnapshot: async () => snapshot,
    onSnapshot: async (listener) => { listeners.add(listener); return () => { listeners.delete(listener) } },
    async resolveClose({ requestId: id, action, remember }) {
      if (id !== snapshot.closeRequestId) return
      if (action !== 'cancel' && remember) {
        const current = await settings.getSettings()
        await settings.updateSettings({ general: { ...current.settings.general, closeBehavior: action } })
      }
      publish({ closeRequestId: null, quitting: false })
    },
    quit: async () => { publish({ closeRequestId: null, quitting: false }) },
    cancelQuit: async () => { publish({ quitting: false }) },
    confirmQuit: async () => undefined,
    onDesktopAction: async () => () => undefined,
    setDesktopReady: async () => undefined,
    async requestClose() {
      if (snapshot.closeRequestId !== null) return
      const preference = (await settings.getSettings()).settings.general.closeBehavior
      if (!preference || preference === 'ask' || (preference === 'background' && !snapshot.backgroundAvailable)) publish({ closeRequestId: ++requestId })
    },
  }
  return client
}
