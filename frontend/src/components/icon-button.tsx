import type { ComponentProps } from 'react'
import { Button } from '@/components/ui/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'

type IconButtonProps = Omit<ComponentProps<typeof Button>, 'aria-label' | 'children'> & {
  /** The accessible name; also the tooltip text unless `tip` says more. */
  label: string
  /** Tooltip text when it differs from the name (shortcuts, "尚未实现"). */
  tip?: string
  children: React.ReactNode
}

/**
 * An icon-only button whose name shows in a tooltip. A disabled button gets no
 * pointer events, so its tooltip hangs off a focusable wrapper instead.
 */
export function IconButton({ label, tip, children, disabled, variant = 'ghost', size = 'icon', ...props }: IconButtonProps) {
  const button = (
    <Button variant={variant} size={size} aria-label={label} disabled={disabled} {...props}>
      {children}
    </Button>
  )
  // Base UI keeps the trigger element it first registered, so switching
  // between the disabled wrapper and the button needs a fresh tooltip.
  return (
    <Tooltip key={disabled ? 'disabled' : 'enabled'}>
      {disabled ? (
        <TooltipTrigger
          render={<span tabIndex={0} className="inline-flex rounded-md focus-visible:ring-3 focus-visible:ring-ring/50 focus-visible:outline-none" />}
        >
          {button}
        </TooltipTrigger>
      ) : (
        <TooltipTrigger render={button} />
      )}
      <TooltipContent>{tip ?? label}</TooltipContent>
    </Tooltip>
  )
}
