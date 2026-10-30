import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { ComputerIcon } from '@hugeicons/core-free-icons'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { cn } from '@/lib/utils'

/** Marks a sent copy that exists only on this device: the server has none yet. */
export function LocalOnlyMarker({ className }: { className?: string }) {
  useTranslation()
  return (
    <Tooltip>
      <TooltipTrigger
        render={<span role="img" aria-label={t('mail:localOnly')} data-local-only className={cn('inline-flex shrink-0 text-muted-foreground', className)} />}
      >
        <HugeiconsIcon icon={ComputerIcon} className="size-3.5" />
      </TooltipTrigger>
      <TooltipContent>{t('mail:localOnlyTip')}</TooltipContent>
    </Tooltip>
  )
}
