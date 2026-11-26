import { invoke, isTauri } from '@tauri-apps/api/core'
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow'
import type { ApiResult } from '@/generated/accounts'
import type { CommandId, MenuGroup } from './model'

export type MenuWindow = { label: string; title: string; focused: boolean }

export async function listMenuWindows() {
  const result = await invoke<ApiResult<MenuWindow[]>>('app_menu_windows')
  return result.ok ? result.data : []
}

export function updateNativeMenu(groups: MenuGroup[]) {
  return invoke('app_menu_update', { groups })
}

export function focusMenuWindow(label: string) {
  return invokeMenuCommand('app_menu_focus_window', { label })
}

export function forwardMenuCommand(id: CommandId) {
  return invokeMenuCommand('app_menu_main_action', { id })
}

async function invokeMenuCommand(command: string, args: Record<string, string>) {
  const result = await invoke<ApiResult<null>>(command, args)
  if (!result.ok) throw result.error
}

export function currentMenuWindow() {
  return isTauri() ? getCurrentWebviewWindow().label : 'main'
}

export function listenToMenuEvent<T>(event: string, listener: (payload: T) => void, onSubscribed?: () => void) {
  if (!isTauri()) return () => undefined
  let disposed = false
  let unlisten: (() => void) | undefined
  void getCurrentWebviewWindow().listen<T>(event, ({ payload }) => listener(payload))
    .then((stop) => {
      if (disposed) { stop(); return }
      unlisten = stop
      onSubscribed?.()
    }, () => undefined)
  return () => {
    disposed = true
    unlisten?.()
  }
}
