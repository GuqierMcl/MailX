import type { ComponentProps } from 'react'
import { cn } from '@/lib/utils'

/** A raised content surface that floats over the app canvas. */
export function Island({ className, ...props }: ComponentProps<'div'>) {
  return (
    <div
      data-slot="island"
      className={cn(
        'relative flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden rounded-2xl bg-background text-foreground shadow-sm ring-1 ring-foreground/10',
        className,
      )}
      {...props}
    />
  )
}
