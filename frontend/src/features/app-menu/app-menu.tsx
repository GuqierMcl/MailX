import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { isTauri } from '@tauri-apps/api/core'
import { ArrowDown01Icon } from '@hugeicons/core-free-icons'
import { HugeiconsIcon } from '@hugeicons/react'
import {
  DropdownMenu, DropdownMenuCheckboxItem, DropdownMenuContent, DropdownMenuItem,
  DropdownMenuSeparator, DropdownMenuSub, DropdownMenuSubContent, DropdownMenuSubTrigger, DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import type { WindowChrome, WindowHost } from '@/features/window-chrome/window-host'
import { windowRole } from '@/lib/composer-window'
import { cachedAppearance } from '@/lib/appearance'
import { toast } from '@/components/ui/toast'
import { errorText } from '@/i18n/errors'
import { invokeAppMenuCommand, menuModalSelector as blockedSelector, useAppMenuState, useProvideSettingsShortcut } from './commands'
import {
  currentMenuWindow, focusMenuWindow, forwardMenuCommand, listMenuWindows,
  listenToMenuEvent, updateNativeMenu, type MenuWindow,
} from './desktop'
import { canEdit, editContext, runEdit } from './edit'
import { accelerators, definitions, editCommands, themes, type CommandId, type GroupId, type MenuGroup, type MenuItem } from './model'

const mainCommands: ReadonlySet<string> = new Set([
  'about', 'update', 'settings', 'accounts', 'quit', 'compose', ...themes, 'guide', 'shortcuts',
])
const editIds: ReadonlySet<string> = new Set(editCommands)
const groupIds = Object.keys(definitions) as GroupId[]
const commandIds: ReadonlySet<string> = new Set([...Object.values(definitions).flat(), ...themes])
const macItems = ['services', 'hide', 'hideOthers', 'showAll', 'bringAllToFront'] as const
const previewAction = 'mailx-preview-app-menu-action'
const inMailList = (target: Element | null) => Boolean(target?.closest('[data-slot="workbench-list"]'))
const allowedWhileBlocked: ReadonlySet<string> = new Set(['quit', 'minimize', 'maximize', 'toggleFullscreen', 'mainWindow'])

function commandBlocked(id: string, modalOpen: boolean) {
  return modalOpen && !allowedWhileBlocked.has(id) && !editIds.has(id) && !id.startsWith('window:')
}

function previewMainWindow(): Window | null {
  try {
    const opener = window.opener as Window | null
    return opener?.location.origin === window.location.origin ? opener : null
  } catch {
    return null
  }
}

export function AppMenu({ host, chrome }: { host: WindowHost | null; chrome: WindowChrome | null }) {
  const { t } = useTranslation('appMenu')
  const commands = useAppMenuState()
  const desktop = chrome !== null && isTauri()
  const composer = windowRole().kind === 'composer'
  const opener = composer && !desktop ? previewMainWindow() : null
  const mac = chrome?.platform === 'macos'
  const nativeMac = desktop && mac && !host?.preview
  const settingsShortcut = chrome?.platform === 'windows' && (desktop || host?.preview === true) ||
    mac && !nativeMac && host?.preview === true
  useProvideSettingsShortcut(nativeMac || settingsShortcut)
  const [windows, setWindows] = useState<MenuWindow[]>([])
  const [subscribed, setSubscribed] = useState(false)
  const [open, setOpen] = useState(false)
  const [modalOpen, setModalOpen] = useState(false)
  const [edit, setEdit] = useState(() => ({ ...editContext(null), inMailList: false }))
  const restoreTo = useRef<Element | null>(null)
  const selectedEdit = useRef(false)

  useEffect(() => {
    const update = () => setModalOpen(document.querySelector(blockedSelector) !== null)
    const observer = new MutationObserver(update)
    observer.observe(document.body, { childList: true, subtree: true, attributes: true, attributeFilter: ['inert', 'aria-modal', 'data-open'] })
    update()
    return () => observer.disconnect()
  }, [])

  useEffect(() => {
    if (!desktop || !chrome?.platform) return
    let active = true
    let request = 0
    const refreshWindows = () => {
      const revision = ++request
      void listMenuWindows().then((list) => { if (active && revision === request) setWindows(list) }, () => undefined)
    }
    refreshWindows()
    window.addEventListener('focus', refreshWindows)
    const stop = listenToMenuEvent('mailx://app-menu-windows-changed', refreshWindows)
    return () => {
      active = false
      window.removeEventListener('focus', refreshWindows)
      stop()
    }
  }, [desktop, chrome?.platform])

  useEffect(() => {
    if ((!mac && chrome?.platform !== 'windows') || open) return
    const update = () => {
      const active = document.activeElement
      if (active?.closest('[data-app-menu-trigger]')) return
      restoreTo.current = active
      setEdit({ ...editContext(active), inMailList: inMailList(active) })
    }
    update()
    document.addEventListener('focusin', update)
    document.addEventListener('selectionchange', update)
    document.addEventListener('input', update)
    return () => {
      document.removeEventListener('focusin', update)
      document.removeEventListener('selectionchange', update)
      document.removeEventListener('input', update)
    }
  }, [chrome?.platform, mac, open])

  function makeItem(id: CommandId): MenuItem {
    const command = commands[id]
    const delegated = composer && (desktop || opener !== null) && mainCommands.has(id)
    const windowAction = chrome !== null && ['close', 'minimize', 'maximize', 'toggleFullscreen'].includes(id)
    const label = id === 'maximize' && mac ? t('zoom')
      : id === 'maximize' && chrome?.state.maximized ? t('restore')
      : id === 'toggleFullscreen' && chrome?.state.fullscreen ? t('exitFullscreen')
      : id === 'toggleSidebar' && command?.checked ? t('hideSidebar')
      : id === 'toggleReader' && command?.checked ? t('hideReader')
      : id === 'toggleAssistant' && command?.checked ? t('hideAssistant') : t(id)
    const listSelectAll = id === 'selectAll' && !edit.target && Boolean(command) && edit.inMailList
    const editOverride = listSelectAll ||
      ((id === 'undo' || id === 'redo') && Boolean(edit.target?.closest('.tiptap')))
    const editEnabled = listSelectAll ? command?.enabled !== false : editIds.has(id) && canEdit(id as typeof editCommands[number], edit)
    return {
      id,
      label: command?.label ?? label,
      enabled: commandBlocked(id, modalOpen) ? false : editIds.has(id) ? (mac && !editOverride) || editEnabled
        : command ? command.enabled !== false
        : delegated || windowAction || (id === 'mainWindow' && desktop),
      checked: command?.checked ?? (composer && themes.some((theme) => theme === id)
        ? id.toLowerCase() === `theme${cachedAppearance().theme}` : undefined),
      accelerator: (mac && (id === 'toggleSidebar' || id === 'toggleAssistant')) ||
        (!nativeMac && (id === 'close' || id === 'quit' || id === 'minimize')) ||
        (id === 'settings' && !nativeMac && !settingsShortcut) ? undefined : accelerators[id],
      native: editOverride ? false : undefined,
    }
  }

  const groups: MenuGroup[] = groupIds.map((id) => ({
    id,
    label: t(id),
    items: definitions[id].map((item): MenuItem => {
      if (item === 'appearance') return { id: item, label: t(item), enabled: true, items: themes.map(makeItem) }
      if (item === 'openWindows') return {
        id: item,
        label: t(item),
        enabled: windows.length > 0,
        items: windows.map((window) => ({
          id: `window:${window.label}`, label: window.title || t('mainWindow'), enabled: true,
          checked: window.label === currentMenuWindow(),
        })),
      }
      return makeItem(item)
    }),
  }))
  if (mac) {
    const appGroup = groups.find(({ id }) => id === 'mailx')
    appGroup?.items.splice(-1, 0, ...macItems.slice(0, 4).map((id) => ({ id, label: t(id), enabled: true })))
    groups.find(({ id }) => id === 'window')?.items.push({ id: 'bringAllToFront', label: t('bringAllToFront'), enabled: true })
  }
  const nativeModel = JSON.stringify(groups)

  useEffect(() => {
    if (!desktop || !mac || !subscribed) return
    void updateNativeMenu(JSON.parse(nativeModel) as MenuGroup[]).catch(() => undefined)
  }, [desktop, mac, subscribed, nativeModel])

  function execute(id: string) {
    if (!commandIds.has(id) && !id.startsWith('window:')) return
    if (commandBlocked(id, document.querySelector(blockedSelector) !== null)) return
    if (editIds.has(id)) {
      if (id === 'selectAll' && !editContext(restoreTo.current).target && commands.selectAll && inMailList(restoreTo.current)) {
        invokeAppMenuCommand(id)
        return
      }
      const target = restoreTo.current
      const run = () => void runEdit(id as typeof editCommands[number], target).catch((error: unknown) => toast.error(errorText(error)))
      if (open) {
        selectedEdit.current = true
        requestAnimationFrame(run)
      } else run()
      return
    }
    if (id.startsWith('window:') || id === 'mainWindow') {
      if (desktop) void focusMenuWindow(id === 'mainWindow' ? 'main' : id.slice('window:'.length)).catch((error: unknown) => toast.error(errorText(error)))
      return
    }
    const command = id as CommandId
    if (commands[command]) {
      invokeAppMenuCommand(command)
      return
    }
    if (composer && desktop && mainCommands.has(id)) {
      void forwardMenuCommand(command).catch((error: unknown) => toast.error(errorText(error)))
      return
    }
    if (opener && mainCommands.has(id)) {
      opener.dispatchEvent(new CustomEvent(previewAction, { detail: id }))
      opener.focus()
      return
    }
    if (id === 'maximize') host?.perform('toggleMaximize')
    if (id === 'toggleFullscreen' || id === 'minimize' || id === 'close') host?.perform(id)
  }

  const executeRef = useRef(execute)
  useEffect(() => { executeRef.current = execute })
  useEffect(() => {
    if (!desktop || !mac) return
    return listenToMenuEvent<string>('mailx://app-menu-action', (id) => executeRef.current(id), () => setSubscribed(true))
  }, [desktop, mac])
  useEffect(() => {
    if (desktop || composer) return
    const onPreviewAction = (event: Event) => {
      const id: unknown = (event as CustomEvent<unknown>).detail
      if (typeof id === 'string' && mainCommands.has(id)) executeRef.current(id)
    }
    window.addEventListener(previewAction, onPreviewAction)
    return () => window.removeEventListener(previewAction, onPreviewAction)
  }, [desktop, composer])
  useEffect(() => {
    if (!settingsShortcut) return
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing || event.repeat || event.altKey || event.shiftKey || event.key !== ',') return
      if (mac ? !event.metaKey || event.ctrlKey : !event.ctrlKey || event.metaKey) return
      if (commandBlocked('settings', document.querySelector(blockedSelector) !== null)) return
      if (commands.settings ? commands.settings.enabled === false : !composer || (!desktop && !previewMainWindow())) return
      event.preventDefault()
      executeRef.current('settings')
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [settingsShortcut, mac, commands.settings, composer, desktop])

  if (chrome?.platform !== 'windows') return null
  return (
    <DropdownMenu modal={false} open={open} onOpenChange={(next) => {
      if (next) {
        selectedEdit.current = false
        setEdit({ ...editContext(restoreTo.current), inMailList: inMailList(restoreTo.current) })
      }
      setOpen(next)
    }}>
      <DropdownMenuTrigger
        aria-label={t('trigger')} title={t('trigger')} data-app-menu-trigger
        className="inline-flex w-[46px] shrink-0 items-center justify-center text-foreground outline-offset-[-2px] hover:bg-foreground/6 active:bg-foreground/4 group-data-[focused=false]/title-bar:text-muted-foreground"
        onPointerDown={(event) => {
          if (event.button === 0 && document.activeElement !== event.currentTarget) restoreTo.current = document.activeElement
        }}
      >
        <HugeiconsIcon icon={ArrowDown01Icon} aria-hidden className="size-3.5" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-36" finalFocus={() => {
        return !selectedEdit.current
      }}>
        {groups.map((group) => <MenuEntry key={group.id} item={{ ...group, enabled: true }} execute={execute} />)}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}

function MenuEntry({ item, execute }: { item: MenuItem; execute: (id: string) => void }) {
  if (item.items) return (
    <DropdownMenuSub>
      <DropdownMenuSubTrigger disabled={!item.enabled}>{item.label}</DropdownMenuSubTrigger>
      <DropdownMenuSubContent side="left" className="min-w-44">
        {item.items.map((child) => <MenuEntry key={child.id} item={child} execute={execute} />)}
      </DropdownMenuSubContent>
    </DropdownMenuSub>
  )
  const shortcut = item.accelerator?.replace('CmdOrCtrl', 'Ctrl').replace('Comma', ',')
  const props = { disabled: !item.enabled, onClick: () => execute(item.id), 'aria-keyshortcuts': shortcut }
  const content = <>
    {item.label}
    {shortcut && <span className="ml-auto pl-4 text-muted-foreground" aria-hidden>{shortcut}</span>}
  </>
  return <>
    {item.separatorBefore && <DropdownMenuSeparator />}
    {item.checked === undefined ? <DropdownMenuItem {...props}>{content}</DropdownMenuItem>
      : <DropdownMenuCheckboxItem checked={item.checked} {...props}>{content}</DropdownMenuCheckboxItem>}
  </>
}
