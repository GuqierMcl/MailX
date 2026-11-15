import { useEffect, useRef } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import { Tick02Icon } from '@hugeicons/core-free-icons'
import { cn } from '@/lib/utils'

type CustomColorSwatchProps = {
  /** The color to show and to start the picker from; only used while `checked`. */
  value: string
  /** Whether the current color is a custom one rather than a preset. */
  checked: boolean
  /** Names the swatch and its picker. */
  label: string
  /** Called on every step while the picker is being dragged. */
  onChange: (color: string) => void
  /** Called once when the picker settles (native `change`), for choices that are saved. */
  onCommit?: (color: string) => void
  /** Size and ring styling; the caller matches the swatches around it. */
  className?: string
  disabled?: boolean
}

/**
 * A rainbow swatch that opens the system color picker, then shows the picked
 * color as selected. The native picker sits invisibly over the swatch so a
 * click opens the OS panel. React's `onChange` follows every drag tick; the
 * native `change` event fires when the picker settles.
 */
export function CustomColorSwatch({ value, checked, label, onChange, onCommit, className, disabled }: CustomColorSwatchProps) {
  const input = useRef<HTMLInputElement>(null)
  const commit = useRef(onCommit)
  useEffect(() => {
    commit.current = onCommit
  })
  useEffect(() => {
    const element = input.current
    if (!element) return
    const settle = () => commit.current?.(element.value.toLowerCase())
    element.addEventListener('change', settle)
    return () => element.removeEventListener('change', settle)
  }, [])
  return (
    <label title={label} data-checked={checked || undefined}
      className={cn('relative flex cursor-pointer items-center justify-center rounded-full outline-none transition-shadow has-focus-visible:ring-3 has-focus-visible:ring-ring/50',
        !checked && 'bg-[conic-gradient(#ef4444,#eab308,#22c55e,#06b6d4,#3b82f6,#a855f7,#ef4444)]',
        disabled && 'cursor-default opacity-50',
        className)}
      style={checked ? { backgroundColor: value } : undefined}>
      <input ref={input} type="color" aria-label={label} disabled={disabled} className="absolute inset-0 size-full cursor-pointer opacity-0"
        value={checked ? value : '#2563eb'} onChange={(event) => onChange(event.target.value.toLowerCase())} />
      {checked && <span className="pointer-events-none"><HugeiconsIcon icon={Tick02Icon} strokeWidth={3} className="size-3.5 text-white mix-blend-difference" /></span>}
    </label>
  )
}
