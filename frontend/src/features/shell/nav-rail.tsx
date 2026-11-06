import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { useRef, type KeyboardEvent, type PointerEvent, type ReactNode } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import { SidebarLeft01Icon, SidebarLeftIcon } from '@hugeicons/core-free-icons'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { NAV_DEFAULT, NAV_MAX, NAV_MIN, NAV_RAIL, NAV_SNAP, clampNavWidth } from './nav-width'

/** Where the sidebar is headed: its full width, and whether it is folded to the rail. */
export type NavSize = { width: number; folded: boolean }

type NavRailProps = {
  /** Folded to the icon rail rather than showing the full sidebar. */
  folded: boolean
  label: string
  width: number
  resizing: boolean
  onResizeStart: () => void
  onResize: (size: NavSize) => void
  /** Called once a drag or keyboard adjustment settles, with the final size. */
  onResizeEnd: (size: NavSize) => void
  /** The full sidebar. */
  children: ReactNode
  /** The folded sidebar: one column of icons. */
  rail: ReactNode
}

/**
 * The navigation column on the canvas, either the full sidebar or its icon
 * rail. The 0.5rem gutter on the right doubles as the resize handle: dragging
 * the sidebar below `NAV_SNAP` folds it, and dragging the rail past it opens it.
 */
export function NavRail({ folded, label, width, resizing, onResizeStart, onResize, onResizeEnd, children, rail }: NavRailProps) {
  useTranslation()
  const drag = useRef<{ x: number; from: number; last: NavSize } | null>(null)

  const start = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return
    event.preventDefault()
    event.currentTarget.setPointerCapture(event.pointerId)
    drag.current = { x: event.clientX, from: folded ? NAV_RAIL : width, last: { width, folded } }
    onResizeStart()
  }
  const move = (event: PointerEvent<HTMLDivElement>) => {
    if (!drag.current) return
    const raw = drag.current.from + event.clientX - drag.current.x
    // Folding keeps the width the sidebar opens at again.
    drag.current.last = raw < NAV_SNAP ? { width, folded: true } : { width: clampNavWidth(raw), folded: false }
    onResize(drag.current.last)
  }
  const end = () => {
    const finished = drag.current
    if (!finished) return
    drag.current = null
    onResizeEnd(finished.last)
  }
  const key = (event: KeyboardEvent<HTMLDivElement>) => {
    const step = event.shiftKey ? 32 : 8
    let next: NavSize
    if (folded) {
      if (event.key === 'ArrowRight') next = { width, folded: false }
      else if (event.key === 'End') next = { width: NAV_MAX, folded: false }
      else return
    } else if (event.key === 'ArrowLeft') next = width <= NAV_MIN ? { width, folded: true } : { width: clampNavWidth(width - step), folded: false }
    else if (event.key === 'ArrowRight') next = { width: clampNavWidth(width + step), folded: false }
    else if (event.key === 'Home') next = { width: NAV_MIN, folded: false }
    else if (event.key === 'End') next = { width: NAV_MAX, folded: false }
    else return
    event.preventDefault()
    onResize(next)
  }
  const keyUp = (event: KeyboardEvent<HTMLDivElement>) => {
    if (['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) onResizeEnd({ width, folded })
  }

  return (
    <nav
      aria-label={label}
      data-slot="nav-rail"
      data-state={folded ? 'collapsed' : 'expanded'}
      style={{ width: (folded ? NAV_RAIL : width) + 8 }}
      className={cn(
        'relative shrink-0 text-sidebar-foreground transition-[width] duration-200 ease-linear motion-reduce:transition-none',
        resizing && 'transition-none',
      )}
    >
      <div style={{ width: folded ? NAV_RAIL : width }} className="flex h-full flex-col overflow-hidden">
        {folded ? rail : children}
      </div>
      <div
        role="separator"
        aria-orientation="vertical"
        aria-label={t('common:resizeSidebar')}
        aria-valuemin={NAV_RAIL}
        aria-valuemax={NAV_MAX}
        aria-valuenow={folded ? NAV_RAIL : width}
        tabIndex={0}
        className="group/handle absolute inset-y-0 right-0 z-10 w-2 cursor-col-resize touch-none outline-hidden"
        onPointerDown={start}
        onPointerMove={move}
        onPointerUp={end}
        onPointerCancel={end}
        onLostPointerCapture={end}
        onDoubleClick={() => {
          onResize({ width: NAV_DEFAULT, folded: false })
          onResizeEnd({ width: NAV_DEFAULT, folded: false })
        }}
        onKeyDown={key}
        onKeyUp={keyUp}
      >
        <div className="mx-auto h-full w-px transition-colors group-hover/handle:bg-foreground/20 group-focus-visible/handle:bg-ring group-active/handle:bg-foreground/30" />
      </div>
    </nav>
  )
}

export function NavRailTrigger({ open, onToggle }: { open: boolean; onToggle: () => void }) {
  useTranslation()
  return (
    <Button variant="ghost" size="icon-sm" aria-label={open ? t('common:collapseSidebar') : t('common:expandSidebar')} onClick={onToggle}>
      <HugeiconsIcon icon={open ? SidebarLeftIcon : SidebarLeft01Icon} strokeWidth={2} />
    </Button>
  )
}
