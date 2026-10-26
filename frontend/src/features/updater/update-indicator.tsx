import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { CircleArrowUp01Icon } from '@hugeicons/core-free-icons'
import { Button } from '@/components/ui/button'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'
import { TitleBarActions } from '@/features/window-chrome/title-bar'
import { downloadPercent, type UpdateSnapshot } from './types'
import { indicatorLabel } from './details'

/**
 * A title-bar button that appears only while there is something to say about
 * an update: found, downloading, or ready to install. Same size as the theme
 * toggle, next to it on the left, never expanding.
 */
export function UpdateIndicator({ snapshot, onOpen }: { snapshot: UpdateSnapshot; onOpen: () => void }) {
  useTranslation()
  const label = indicatorLabel(snapshot)
  if (!label) return null
  // A notify-only edition never downloads: it only ever shows the found state.
  const phase = snapshot.canInstall ? snapshot.phase : 'available'
  const percent = downloadPercent(snapshot)
  return (
    <TitleBarActions side="trailing" order={-1}>
      <TooltipProvider>
        <Tooltip>
          <TooltipTrigger
            render={<Button variant="ghost" size="icon-sm" aria-label={label} onClick={onOpen} />}
          >
            {phase === 'downloading'
              ? <ProgressRing percent={percent ?? 0} />
              : (
                <span className="relative flex">
                  <HugeiconsIcon icon={CircleArrowUp01Icon} strokeWidth={2} className={phase === 'ready' ? 'text-primary' : undefined} />
                  {phase === 'available' && (
                    <span aria-hidden className="absolute -top-0.5 -right-0.5 size-1.5 rounded-full bg-primary ring-2 ring-background" />
                  )}
                </span>
              )}
          </TooltipTrigger>
          <TooltipContent side="bottom">{label}</TooltipContent>
        </Tooltip>
      </TooltipProvider>
    </TitleBarActions>
  )
}

/** A ring in the same box as the icon, filled to the download's share. No animation. */
function ProgressRing({ percent }: { percent: number }) {
  const radius = 6
  const circumference = 2 * Math.PI * radius
  return (
    <svg viewBox="0 0 16 16" aria-hidden className="size-4 -rotate-90">
      <circle cx="8" cy="8" r={radius} fill="none" strokeWidth="2" className="stroke-muted" />
      <circle
        cx="8" cy="8" r={radius} fill="none" strokeWidth="2" strokeLinecap="round"
        strokeDasharray={circumference}
        strokeDashoffset={circumference * (1 - Math.min(100, Math.max(0, percent)) / 100)}
        className="stroke-primary"
      />
    </svg>
  )
}
