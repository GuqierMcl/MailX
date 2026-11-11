import { useEffect, useState } from 'react'

const QUERY = '(prefers-reduced-motion: reduce)'

/** True while the system asks for reduced motion; follows changes. */
export function usePrefersReducedMotion() {
  const [reduced, setReduced] = useState(() => globalThis.matchMedia?.(QUERY).matches ?? false)
  useEffect(() => {
    const query = globalThis.matchMedia?.(QUERY)
    if (!query?.addEventListener) return
    const onChange = () => setReduced(query.matches)
    onChange()
    query.addEventListener('change', onChange)
    return () => query.removeEventListener('change', onChange)
  }, [])
  return reduced
}
