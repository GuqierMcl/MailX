import { useDroppable } from '@dnd-kit/react'
import { Button } from '@/components/ui/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import type { RecipientDropData } from './recipient-field'

type RowToggleProps = {
  row: 'cc' | 'bcc'
  label: string
  /** Tooltip with the shortcut. */
  tip: string
  disabled?: boolean
  onOpen: () => void
}

/** "抄送" / "密送" at the end of the To row. A chip dropped on it opens that row with the chip in it. */
export function RowToggle({ row, label, tip, disabled, onOpen }: RowToggleProps) {
  const { ref, isDropTarget } = useDroppable<RecipientDropData>({ id: `recipient-toggle-${row}`, data: { row }, disabled, collisionPriority: 10 })
  return (
    <Tooltip>
      <TooltipTrigger
        render={(
          <Button ref={ref} type="button" variant="ghost" size="xs" disabled={disabled} onClick={onOpen}
            data-drop-target={isDropTarget || undefined} className="h-6 px-1.5 text-xs font-normal text-muted-foreground hover:text-foreground data-drop-target:bg-muted data-drop-target:text-foreground" />
        )}
      >
        {label}
      </TooltipTrigger>
      <TooltipContent>{tip}</TooltipContent>
    </Tooltip>
  )
}
