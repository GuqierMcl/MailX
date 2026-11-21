import { useLayoutEffect, useRef, useSyncExternalStore } from 'react'
import { toast } from '@/components/ui/toast'
import { errorText } from '@/i18n/errors'
import type { CommandId } from './model'

export const menuModalSelector = '[data-main-interaction][inert], [role="dialog"][data-open], [role="dialog"][aria-modal="true"], [role="alertdialog"][data-open]'

export type AppMenuCommand = {
  run: () => void | Promise<unknown>
  enabled?: boolean
  checked?: boolean
  label?: string
}

type Commands = Partial<Record<CommandId, AppMenuCommand>>
const registrations = new Map<object, Commands>()
const listeners = new Set<() => void>()
let snapshot: Commands = {}

function publish() {
  const current = Object.assign({}, ...registrations.values()) as Commands
  const ids = new Set([...Object.keys(current), ...Object.keys(snapshot)] as CommandId[])
  for (const id of ids) {
    const next = current[id]
    const previous = snapshot[id]
    if (Boolean(next) !== Boolean(previous) || next?.enabled !== previous?.enabled ||
        next?.checked !== previous?.checked || next?.label !== previous?.label) {
      snapshot = current
      listeners.forEach((listener) => listener())
      return
    }
  }
}

function subscribe(listener: () => void) {
  listeners.add(listener)
  return () => { listeners.delete(listener) }
}

export function useAppMenuState() {
  return useSyncExternalStore(subscribe, () => snapshot)
}

export function useAppMenuCommands(commands: Commands) {
  const owner = useRef({})
  useLayoutEffect(() => {
    registrations.set(owner.current, commands)
    publish()
  })
  useLayoutEffect(() => {
    const registration = owner.current
    return () => {
      registrations.delete(registration)
      publish()
    }
  }, [])
}

export function invokeAppMenuCommand(id: CommandId) {
  for (const commands of [...registrations.values()].reverse()) {
    const command = commands[id]
    if (!command) continue
    if (command.enabled === false) return false
    try {
      void Promise.resolve(command.run()).catch((error: unknown) => toast.error(errorText(error)))
    } catch (error) {
      toast.error(errorText(error))
    }
    return true
  }
  return false
}

// Whether this window's app menu handles Cmd/Ctrl+, for Settings.
let settingsShortcut = false
const settingsShortcutListeners = new Set<() => void>()

function subscribeSettingsShortcut(listener: () => void) {
  settingsShortcutListeners.add(listener)
  return () => { settingsShortcutListeners.delete(listener) }
}

export function useSettingsShortcutAvailable() {
  return useSyncExternalStore(subscribeSettingsShortcut, () => settingsShortcut)
}

export function useProvideSettingsShortcut(available: boolean) {
  useLayoutEffect(() => {
    settingsShortcut = available
    settingsShortcutListeners.forEach((listener) => listener())
    return () => {
      settingsShortcut = false
      settingsShortcutListeners.forEach((listener) => listener())
    }
  }, [available])
}
