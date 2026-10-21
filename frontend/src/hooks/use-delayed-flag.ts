import { useEffect, useRef, useState } from 'react'

/**
 * Smooths a loading flag so a fast load never flashes a placeholder: the flag
 * only turns on after `delay`, and once on it stays on for at least `minimum`.
 * Local data usually arrives within a few tens of milliseconds, so most loads
 * never show a placeholder at all.
 */
export function useDelayedFlag(active: boolean, delay = 150, minimum = 300) {
  const [shown, setShown] = useState(false)
  const since = useRef(0)

  useEffect(() => {
    if (active && !shown) {
      const timer = setTimeout(() => {
        since.current = Date.now()
        setShown(true)
      }, delay)
      return () => clearTimeout(timer)
    }
    if (!active && shown) {
      const timer = setTimeout(() => setShown(false), Math.max(0, minimum - (Date.now() - since.current)))
      return () => clearTimeout(timer)
    }
  }, [active, shown, delay, minimum])

  return shown
}
