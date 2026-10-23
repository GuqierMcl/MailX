import { useCallback, useEffect, useRef, useState } from 'react'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import { usePrefersReducedMotion } from './use-reduced-motion'

/** Clicks this soon after the confirm state appears are ignored, so a double click cannot skip the confirmation. */
export const CONFIRM_GUARD_MS = 400
/** The confirm state falls back to "发送" after this long without a second press. */
export const CONFIRM_TIMEOUT_MS = 4000

type Stage = 'idle' | 'checking' | 'confirm' | 'sending'

type SendButtonProps = {
  /** An empty subject changes the confirm label. */
  subjectEmpty: boolean
  disabled?: boolean
  /**
   * First press. Validate, write unsaved edits, and return the saved revision to
   * send; `null` means it was refused (the caller shows why) and the button stays.
   */
  onPrepare: () => Promise<number | null>
  /** Second press. Queue the draft at the revision `onPrepare` returned. */
  onConfirm: (expectedRevision: number) => Promise<void>
  /** Any change to this value (an edit, a reload) takes the button back to "发送". */
  resetToken: unknown
}

/**
 * The two-step send button (sending.md section 3): the first press checks and
 * saves, the button then widens and changes color to ask again, the second press
 * sends. It is a guard against slips, not a security boundary.
 */
export function SendButton({ subjectEmpty, disabled, onPrepare, onConfirm, resetToken }: SendButtonProps) {
  const [stage, setStage] = useState<Stage>('idle')
  const reducedMotion = usePrefersReducedMotion()
  const revision = useRef(0)
  const guardUntil = useRef(0)
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined)
  const stageRef = useRef<Stage>('idle')
  const alive = useRef(true)

  const move = useCallback((next: Stage) => {
    stageRef.current = next
    setStage(next)
  }, [])

  const revert = useCallback(() => {
    clearTimeout(timer.current)
    if (stageRef.current === 'confirm') move('idle')
  }, [move])

  useEffect(() => {
    alive.current = true
    return () => {
      alive.current = false
      clearTimeout(timer.current)
    }
  }, [])

  // Any edit or reload, Esc, or the window losing focus withdraws the confirmation.
  const firstToken = useRef(resetToken)
  useEffect(() => {
    if (firstToken.current === resetToken) return
    firstToken.current = resetToken
    revert()
  }, [resetToken, revert])
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => { if (event.key === 'Escape' && stageRef.current === 'confirm') revert() }
    window.addEventListener('keydown', onKeyDown)
    window.addEventListener('blur', revert)
    return () => {
      window.removeEventListener('keydown', onKeyDown)
      window.removeEventListener('blur', revert)
    }
  }, [revert])

  const press = useCallback(async () => {
    const current = stageRef.current
    if (disabled || current === 'checking' || current === 'sending') return
    if (current === 'idle') {
      move('checking')
      const saved = await onPrepare().catch(() => null)
      if (!alive.current) return
      if (saved === null) {
        move('idle')
        return
      }
      revision.current = saved
      guardUntil.current = Date.now() + CONFIRM_GUARD_MS
      move('confirm')
      clearTimeout(timer.current)
      timer.current = setTimeout(revert, CONFIRM_TIMEOUT_MS)
      return
    }
    if (Date.now() < guardUntil.current) return
    clearTimeout(timer.current)
    move('sending')
    try {
      await onConfirm(revision.current)
    } finally {
      if (alive.current) move('idle')
    }
  }, [disabled, move, onConfirm, onPrepare, revert])

  // Cmd/Ctrl+Enter is the same two presses.
  const pressRef = useRef(press)
  useEffect(() => { pressRef.current = press })
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Enter' && (event.metaKey || event.ctrlKey) && !event.altKey && !event.shiftKey && !event.isComposing) {
        event.preventDefault()
        void pressRef.current()
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [])

  const confirming = stage === 'confirm' || stage === 'sending'
  const label = stage === 'sending' ? '正在发送…' : confirming ? (subjectEmpty ? '无主题，确认发送' : '确认发送') : '发送'
  return (
    <>
      <Button
        type="button"
        data-stage={stage}
        aria-disabled={disabled || stage === 'checking' || stage === 'sending' || undefined}
        disabled={disabled}
        onClick={() => void press()}
        className={cn(
          'h-8 justify-center overflow-hidden px-3 text-sm whitespace-nowrap',
          !reducedMotion && 'transition-[width,background-color,color] duration-200 ease-out',
          confirming
            ? cn(subjectEmpty ? 'w-36' : 'w-24', 'bg-emerald-700 text-white hover:bg-emerald-700/90 dark:bg-emerald-600 dark:hover:bg-emerald-600/90')
            : 'w-16',
        )}
      >
        {label}
      </Button>
      <span role="status" aria-live="polite" className="sr-only">{stage === 'confirm' ? '再次按下以确认发送' : ''}</span>
    </>
  )
}
