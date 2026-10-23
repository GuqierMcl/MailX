import { HugeiconsIcon } from '@hugeicons/react'
import { PanelRightCloseIcon, PanelRightOpenIcon } from '@hugeicons/core-free-icons'
import { Button } from '@/components/ui/button'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { TitleBarActions } from '@/features/window-chrome/title-bar'

/**
 * Hides or shows the reader beside the message list. It sits at the far right
 * of the title bar, after the color-mode button (`order` keeps it there however
 * the two mount), and its icon shows what a click will do.
 */
export function ReaderToggle({ collapsed, onToggle }: { collapsed: boolean; onToggle: () => void }) {
  const label = collapsed ? '展开邮件内容' : '收起邮件内容'
  return (
    <TitleBarActions side="trailing" order={1}>
      <Tooltip>
        <TooltipTrigger
          render={(
            <Button variant="ghost" size="icon-sm" aria-label={label} aria-expanded={!collapsed} onClick={onToggle}>
              <HugeiconsIcon icon={collapsed ? PanelRightOpenIcon : PanelRightCloseIcon} />
            </Button>
          )}
        />
        <TooltipContent>{label}</TooltipContent>
      </Tooltip>
    </TitleBarActions>
  )
}
