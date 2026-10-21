import { useRef, type KeyboardEvent, type PointerEvent, type ReactNode } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import { SidebarLeftIcon } from '@hugeicons/core-free-icons'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { NAV_DEFAULT, NAV_MAX, NAV_MIN, clampNavWidth } from './nav-width'

type NavRailProps = {
  open: boolean
  label: string
  width: number
  resizing: boolean
  onResizeStart: () => void
  onResize: (width: number) => void
  /** Called once a drag or keyboard adjustment settles, with the final width. */
  onResizeEnd: (width: number) => void
  children: ReactNode
}

/**
 * The navigation column on the canvas. Collapsed, only the 0.5rem gutter
 * stays so the island keeps its margin from the window edge. The gutter on
 * the right doubles as the resize handle.
 */
export function NavRail({ open, label, width, resizing, onResizeStart, onResize, onResizeEnd, children }: NavRailProps) {
  const drag = useRef<{ x: number; width: number; last: number } | null>(null)

  const start = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return
    event.preventDefault()
    event.currentTarget.setPointerCapture(event.pointerId)
    drag.current = { x: event.clientX, width, last: width }
    onResizeStart()
  }
  const move = (event: PointerEvent<HTMLDivElement>) => {
    if (!drag.current) return
    drag.current.last = clampNavWidth(drag.current.width + event.clientX - drag.current.x)
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
    if (event.key === 'ArrowLeft') onResize(clampNavWidth(width - step))
    else if (event.key === 'ArrowRight') onResize(clampNavWidth(width + step))
    else if (event.key === 'Home') onResize(NAV_MIN)
    else if (event.key === 'End') onResize(NAV_MAX)
    else return
    event.preventDefault()
  }
  const keyUp = (event: KeyboardEvent<HTMLDivElement>) => {
    if (['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) onResizeEnd(width)
  }

  return (
    <nav
      aria-label={label}
      data-slot="nav-rail"
      data-state={open ? 'expanded' : 'collapsed'}
      style={{ width: open ? width + 8 : 8 }}
      className={cn(
        'relative shrink-0 text-sidebar-foreground transition-[width] duration-200 ease-linear motion-reduce:transition-none',
        resizing && 'transition-none',
      )}
    >
      <div
        inert={!open}
        style={{ width }}
        className={cn('flex h-full flex-col overflow-hidden transition-opacity duration-200', !open && 'opacity-0')}
      >
        {children}
      </div>
      {open && (
        <div
          role="separator"
          aria-orientation="vertical"
          aria-label="调整侧边栏宽度"
          aria-valuemin={NAV_MIN}
          aria-valuemax={NAV_MAX}
          aria-valuenow={width}
          tabIndex={0}
          className="group/handle absolute inset-y-0 right-0 z-10 w-2 cursor-col-resize touch-none outline-hidden"
          onPointerDown={start}
          onPointerMove={move}
          onPointerUp={end}
          onPointerCancel={end}
          onLostPointerCapture={end}
          onDoubleClick={() => {
            onResize(NAV_DEFAULT)
            onResizeEnd(NAV_DEFAULT)
          }}
          onKeyDown={key}
          onKeyUp={keyUp}
        >
          <div className="mx-auto h-full w-px transition-colors group-hover/handle:bg-foreground/20 group-focus-visible/handle:bg-ring group-active/handle:bg-foreground/30" />
        </div>
      )}
    </nav>
  )
}

export function NavRailTrigger({ open, onToggle }: { open: boolean; onToggle: () => void }) {
  return (
    <Button variant="ghost" size="icon-sm" aria-label="折叠或展开侧边栏" aria-expanded={open} onClick={onToggle}>
      <HugeiconsIcon icon={SidebarLeftIcon} strokeWidth={2} />
    </Button>
  )
}
