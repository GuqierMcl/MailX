import { useCallback, useEffect, useMemo, useState, useSyncExternalStore, type ReactNode } from 'react'
import { TitleBarActions } from '@/features/window-chrome/title-bar'
import { Island } from './island'
import { NAV_RAIL_WINDOW } from './layout-defaults'
import { NavRail, NavRailTrigger, type NavSize } from './nav-rail'
import { ShellContext } from './shell-context'

type ShellLayoutProps = {
  navLabel: string
  /** The full sidebar. */
  nav: ReactNode
  /** The folded sidebar: one column of icons. */
  rail: ReactNode
  /** The saved sidebar width. */
  navWidth: number
  /** The saved choice to fold the sidebar to its rail. */
  navCollapsed: boolean
  /** Page actions beside the sidebar toggle in the title bar. */
  navActions?: ReactNode
  /** Called when the user settles on a width or folds the sidebar; the caller persists it. */
  onNavChange: (change: { navWidth?: number; navCollapsed?: boolean }) => void
  children: ReactNode
}

const NARROW_QUERY = `(max-width: ${NAV_RAIL_WINDOW - 1}px)`

function subscribeNarrow(onChange: () => void) {
  const query = window.matchMedia(NARROW_QUERY)
  query.addEventListener('change', onChange)
  return () => query.removeEventListener('change', onChange)
}

/** Whether the window is too narrow for the full sidebar beside the list and reader. */
function useNarrowWindow() {
  return useSyncExternalStore(subscribeNarrow, () => window.matchMedia(NARROW_QUERY).matches, () => false)
}

/**
 * Canvas layout under the title bar: navigation sits directly on the canvas
 * (the same plane as the title bar) and the content floats as one island.
 *
 * The sidebar is either full or folded to its icon rail, never hidden. The
 * user's choice is saved; a narrow window shows the rail on its own, and opening
 * the sidebar there lasts only until the window widens, leaving the choice as it was.
 */
export function ShellLayout({ navLabel, nav, rail, navWidth, navCollapsed, navActions, onNavChange, children }: ShellLayoutProps) {
  const narrow = useNarrowWindow()
  const [narrowOpen, setNarrowOpen] = useState(false)
  const [wasNarrow, setWasNarrow] = useState(narrow)
  if (wasNarrow !== narrow) {
    // Adjusting state during render: a window that widens forgets the temporary opening.
    setWasNarrow(narrow)
    setNarrowOpen(false)
  }

  const [resizing, setResizing] = useState(false)
  // What the user chose stays on screen until the saved value catches up
  // (`from` is the saved value it was chosen against).
  const [widthDraft, setWidthDraft] = useState<{ from: number; value: number } | null>(null)
  const [foldDraft, setFoldDraft] = useState<{ from: boolean; value: boolean } | null>(null)
  // A drag shows where it is headed before it settles.
  const [dragFolded, setDragFolded] = useState<boolean | null>(null)
  const width = widthDraft && widthDraft.from === navWidth ? widthDraft.value : navWidth
  const chosenFolded = foldDraft && foldDraft.from === navCollapsed ? foldDraft.value : navCollapsed
  const settledFolded = narrow ? !narrowOpen : chosenFolded
  const folded = dragFolded ?? settledFolded

  const setFolded = useCallback((next: boolean) => {
    if (narrow) {
      setNarrowOpen(!next)
      return
    }
    setFoldDraft({ from: navCollapsed, value: next })
    // Against what is shown, not what is saved: a quick fold and unfold must save both.
    if (next !== chosenFolded) onNavChange({ navCollapsed: next })
  }, [narrow, navCollapsed, chosenFolded, onNavChange])
  const toggleNav = useCallback(() => setFolded(!settledFolded), [setFolded, settledFolded])

  const controls = useMemo(() => ({ navOpen: !folded, toggleNav }), [folded, toggleNav])

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'b' && (event.metaKey || event.ctrlKey)) {
        event.preventDefault()
        toggleNav()
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [toggleNav])

  const resize = ({ width: next, folded: nextFolded }: NavSize) => {
    setWidthDraft({ from: navWidth, value: next })
    setDragFolded(nextFolded === settledFolded ? null : nextFolded)
  }
  const resizeEnd = ({ width: next, folded: nextFolded }: NavSize) => {
    setResizing(false)
    setDragFolded(null)
    if (nextFolded !== settledFolded) setFolded(nextFolded)
    if (!nextFolded && next !== navWidth) onNavChange({ navWidth: next })
  }

  return (
    <div data-slot="shell-layout" className="flex h-full min-h-0 pr-2 pb-2">
      <TitleBarActions side="leading">
        <NavRailTrigger open={!folded} onToggle={toggleNav} />
        {navActions}
      </TitleBarActions>
      <NavRail
        folded={folded}
        label={navLabel}
        width={width}
        resizing={resizing}
        onResizeStart={() => setResizing(true)}
        onResize={resize}
        onResizeEnd={resizeEnd}
        rail={rail}
      >
        {nav}
      </NavRail>
      <ShellContext.Provider value={controls}>
        <Island className={resizing ? 'pointer-events-none' : undefined}>{children}</Island>
      </ShellContext.Provider>
    </div>
  )
}
