import { useEffect, useMemo, useState, type ReactNode } from 'react'
import { TitleBarActions } from '@/features/window-chrome/title-bar'
import { Island } from './island'
import { NavRail, NavRailTrigger } from './nav-rail'
import { ShellContext } from './shell-context'

type ShellLayoutProps = {
  navLabel: string
  nav: ReactNode
  /** The saved sidebar width. */
  navWidth: number
  /** Called when the user finishes resizing; the caller persists it. */
  onNavWidthChange: (width: number) => void
  children: ReactNode
}

/**
 * Canvas layout under the title bar: navigation sits directly on the canvas
 * (the same plane as the title bar) and the content floats as one island.
 */
export function ShellLayout({ navLabel, nav, navWidth, onNavWidthChange, children }: ShellLayoutProps) {
  const [navOpen, setNavOpen] = useState(true)
  const [resizing, setResizing] = useState(false)
  // A width the user chose stays on screen until the saved width catches up
  // (`from` is the saved width it was chosen against).
  const [draft, setDraft] = useState<{ from: number; value: number } | null>(null)
  const width = draft && draft.from === navWidth ? draft.value : navWidth

  const controls = useMemo(() => ({ navOpen, toggleNav: () => setNavOpen((open) => !open) }), [navOpen])

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'b' && (event.metaKey || event.ctrlKey)) {
        event.preventDefault()
        setNavOpen((open) => !open)
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [])

  return (
    <div data-slot="shell-layout" className="flex h-full min-h-0 pr-2 pb-2">
      <TitleBarActions side="leading">
        <NavRailTrigger open={navOpen} onToggle={() => setNavOpen((open) => !open)} />
      </TitleBarActions>
      <NavRail
        open={navOpen}
        label={navLabel}
        width={width}
        resizing={resizing}
        onResizeStart={() => setResizing(true)}
        onResize={(value) => setDraft({ from: navWidth, value })}
        onResizeEnd={(value) => {
          setResizing(false)
          if (value !== navWidth) onNavWidthChange(value)
        }}
      >
        {nav}
      </NavRail>
      <ShellContext.Provider value={controls}>
        <Island className={resizing ? 'pointer-events-none' : undefined}>{children}</Island>
      </ShellContext.Provider>
    </div>
  )
}
