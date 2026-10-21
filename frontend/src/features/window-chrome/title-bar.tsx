import { useCallback, useContext, useMemo, useRef, useState, type MouseEvent, type ReactNode } from 'react'
import { createPortal } from 'react-dom'
import { cn } from '@/lib/utils'
import { TitleBarSlotsContext, type TitleBarSide, type TitleBarSlotElements } from './title-bar-slots'
import { useWindowChrome } from './use-window-chrome'
import type { WindowAction, WindowButton, WindowHost, WindowPlatform } from './window-host'

/** Elements that keep their own pointer behaviour instead of moving the window. */
const NO_DRAG_SELECTOR = [
  'button',
  'a[href]',
  'input',
  'textarea',
  'select',
  '[role="button"]',
  '[role="link"]',
  '[role="menuitem"]',
  '[role="tab"]',
  '[contenteditable]:not([contenteditable="false"])',
  '[data-window-no-drag]',
].join(', ')

function isDragSurface(event: MouseEvent<HTMLElement>) {
  if (!(event.target instanceof Element)) return false
  const interactive = event.target.closest(NO_DRAG_SELECTOR)
  return interactive === null || !event.currentTarget.contains(interactive)
}

/** Lets pages place actions into the title bar with `TitleBarActions`, keeping their React context. */
export function TitleBarSlotsProvider({ children }: { children: ReactNode }) {
  const [slots, setSlots] = useState<TitleBarSlotElements>({ leading: null, center: null, trailing: null })
  const register = useCallback((side: TitleBarSide, element: HTMLElement | null) => {
    setSlots((current) => (current[side] === element ? current : { ...current, [side]: element }))
  }, [])
  const value = useMemo(() => ({ slots, register }), [slots, register])
  return <TitleBarSlotsContext.Provider value={value}>{children}</TitleBarSlotsContext.Provider>
}

/**
 * Renders actions into the title bar: `leading` sits right after the macOS
 * traffic lights (or the left edge), `center` fills the space between the two
 * sides, `trailing` just before the window controls.
 */
export function TitleBarActions({ side, children }: { side: TitleBarSide; children: ReactNode }) {
  const target = useContext(TitleBarSlotsContext)?.slots[side]
  return target ? createPortal(children, target) : null
}

type TitleBarProps = {
  host: WindowHost | null
  title?: string
  children?: ReactNode
}

/**
 * Replaces the system title bar. Empty areas drag the window; a double click
 * does what the OS title bar would (zoom, minimize or nothing) because the host
 * reads the system preference. Interactive children never start a drag.
 */
export function TitleBar({ host, title, children }: TitleBarProps) {
  const chrome = useWindowChrome(host)
  const register = useContext(TitleBarSlotsContext)?.register
  const leadingSlot = useCallback((element: HTMLElement | null) => register?.('leading', element), [register])
  const centerSlot = useCallback((element: HTMLElement | null) => register?.('center', element), [register])
  const trailingSlot = useCallback((element: HTMLElement | null) => register?.('trailing', element), [register])
  const pendingDoubleClick = useRef<{ x: number; y: number } | null>(null)
  const platform = chrome?.platform
  const state = chrome?.state
  const perform = (action: WindowAction) => host?.perform(action)

  // Same timing as Tauri's drag region: macOS acts when the second click is
  // released in place, Windows and Linux on the second press.
  const handleMouseDown = (event: MouseEvent<HTMLElement>) => {
    if (!chrome || event.button !== 0 || !isDragSurface(event)) return
    event.preventDefault()
    if (event.detail === 2) {
      if (platform === 'macos') pendingDoubleClick.current = { x: event.clientX, y: event.clientY }
      else perform('titleBarDoubleClick')
      return
    }
    perform('startDragging')
  }

  const handleMouseUp = (event: MouseEvent<HTMLElement>) => {
    const pending = pendingDoubleClick.current
    pendingDoubleClick.current = null
    if (pending && event.detail === 2 && pending.x === event.clientX && pending.y === event.clientY) {
      perform('titleBarDoubleClick')
    }
  }

  const centeredTitle = platform !== 'windows'

  return (
    <header
      data-slot="title-bar"
      data-platform={platform}
      data-focused={state?.focused ?? true}
      className="group/title-bar relative flex h-(--titlebar-height) shrink-0 items-stretch text-sidebar-foreground select-none"
      onMouseDown={handleMouseDown}
      onMouseUp={handleMouseUp}
      onContextMenu={(event) => isDragSurface(event) && event.preventDefault()}
    >
      {platform === 'macos' && !state?.fullscreen && (
        <div data-slot="title-bar-traffic-lights" aria-hidden className="w-[84px] shrink-0" />
      )}
      {platform && chrome.leading.length > 0 && (
        <WindowControls platform={platform} buttons={chrome.leading} maximized={state?.maximized} onAction={perform} />
      )}
      <div ref={leadingSlot} data-slot="title-bar-leading" className="flex shrink-0 items-center gap-1 px-2 empty:px-0" />
      <div className="flex min-w-0 flex-1 items-center gap-2 px-3">
        {title && (
          <span
            data-slot="title-bar-title"
            className={cn(
              'pointer-events-none truncate text-xs font-medium text-muted-foreground group-data-[focused=false]/title-bar:opacity-60',
              centeredTitle && 'absolute left-1/2 -translate-x-1/2',
            )}
          >
            {title}
          </span>
        )}
        {children}
        <div ref={centerSlot} data-slot="title-bar-center" className="flex min-w-0 flex-1 items-center justify-center empty:hidden" />
      </div>
      <div ref={trailingSlot} data-slot="title-bar-trailing" className="flex shrink-0 items-center gap-1 px-2 empty:px-0" />
      {platform && chrome.trailing.length > 0 && (
        <WindowControls platform={platform} buttons={chrome.trailing} maximized={state?.maximized} onAction={perform} />
      )}
    </header>
  )
}

type WindowControlsProps = {
  platform: WindowPlatform
  buttons: WindowButton[]
  maximized?: boolean
  onAction: (action: WindowAction) => void
}

const BUTTON_ACTIONS: Record<WindowButton, WindowAction> = {
  minimize: 'minimize',
  maximize: 'toggleMaximize',
  close: 'close',
}

function buttonLabel(button: WindowButton, maximized?: boolean) {
  if (button === 'minimize') return '最小化'
  if (button === 'close') return '关闭'
  return maximized ? '还原' : '最大化'
}

function WindowControls({ platform, buttons, maximized, onAction }: WindowControlsProps) {
  const windows = platform === 'windows'
  return (
    <div
      data-slot="window-controls"
      className={cn('flex shrink-0', windows ? 'items-stretch' : 'items-center gap-3 px-3')}
    >
      {buttons.map((button) => {
        const label = buttonLabel(button, maximized)
        return (
          <button
            key={button}
            type="button"
            tabIndex={-1}
            aria-label={label}
            title={label}
            data-window-button={button}
            className={windows ? windowsButtonClass(button) : linuxButtonClass}
            onClick={() => onAction(BUTTON_ACTIONS[button])}
          >
            {windows ? <WindowsGlyph button={button} maximized={maximized} /> : <AdwaitaIcon button={button} maximized={maximized} />}
          </button>
        )
      })}
    </div>
  )
}

// Windows 11 caption buttons: 46px wide, Segoe Fluent Icons glyphs, system red for close.
function windowsButtonClass(button: WindowButton) {
  return cn(
    "inline-flex w-[46px] items-center justify-center font-['Segoe_Fluent_Icons','Segoe_MDL2_Assets'] text-[10px] leading-none text-foreground transition-colors duration-100 group-data-[focused=false]/title-bar:text-muted-foreground",
    button === 'close'
      ? 'hover:bg-[#c42b1c] hover:text-white active:bg-[#c42b1c]/90 active:text-white/80 group-data-[focused=false]/title-bar:hover:text-white'
      : 'hover:bg-foreground/6 active:bg-foreground/4',
  )
}

const WINDOWS_GLYPHS = { minimize: '\uE921', maximize: '\uE922', restore: '\uE923', close: '\uE8BB' }

function WindowsGlyph({ button, maximized }: { button: WindowButton; maximized?: boolean }) {
  const glyph = button === 'maximize' && maximized ? WINDOWS_GLYPHS.restore : WINDOWS_GLYPHS[button]
  return <span aria-hidden>{glyph}</span>
}

// libadwaita window controls: round 24px buttons with 16px symbolic icons.
const linuxButtonClass =
  'inline-flex size-6 items-center justify-center rounded-full bg-foreground/10 text-foreground transition-colors hover:bg-foreground/15 active:bg-foreground/25 group-data-[focused=false]/title-bar:opacity-60'

function AdwaitaIcon({ button, maximized }: { button: WindowButton; maximized?: boolean }) {
  return (
    <svg aria-hidden viewBox="0 0 16 16" className="size-4" fill="none" stroke="currentColor" strokeWidth={1.5} strokeLinecap="round">
      {button === 'minimize' && <path d="M4.5 11h7" />}
      {button === 'maximize' && !maximized && <rect x="4.25" y="4.25" width="7.5" height="7.5" rx="1" />}
      {button === 'maximize' && maximized && (
        <>
          <rect x="4.25" y="6.25" width="5.5" height="5.5" rx="1" />
          <path d="M6.5 4.25h4.25a1 1 0 0 1 1 1V9.5" />
        </>
      )}
      {button === 'close' && <path d="M5 5l6 6M11 5l-6 6" />}
    </svg>
  )
}
