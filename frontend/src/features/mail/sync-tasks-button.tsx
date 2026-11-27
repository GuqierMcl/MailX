import { useCallback, useEffect, useState } from 'react'
import { Cancel01Icon, PauseIcon, PlayIcon, RefreshIcon } from '@hugeicons/core-free-icons'
import { HugeiconsIcon } from '@hugeicons/react'
import { useTranslation } from 'react-i18next'
import type { Account, AccountBackfillTask, BackfillControlAction } from '@/generated/accounts'
import { Button } from '@/components/ui/button'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'
import { TitleBarActions } from '@/features/window-chrome/title-bar'
import { errorText } from '@/i18n/errors'
import type { MailxClient } from '@/lib/mailx-client'

const QUIET_SHOW_DELAY_MS = 1500
const RING_RADIUS = 8
const RING_CIRCUMFERENCE = 2 * Math.PI * RING_RADIUS

function aggregateProgress(tasks: AccountBackfillTask[]): number {
  let fetched = 0
  let total = 0
  let hasTotal = false
  for (const task of tasks) {
    if (task.total !== null && task.total > 0) {
      hasTotal = true
      fetched += Math.min(task.fetched, task.total)
      total += task.total
    }
  }
  if (!hasTotal || total <= 0) return 0.25
  return Math.max(0.08, Math.min(1, fetched / total))
}

/**
 * Quiet title-bar button for background historical backfill tasks.
 * Hidden when no backfill is active, delayed for quick running tasks, and
 * rendered with a static progress ring (never a spinning indicator).
 */
export function SyncTasksButton({
  client,
  accounts,
  tasks,
  onTasksChange,
}: {
  client: MailxClient
  accounts: Account[]
  tasks: AccountBackfillTask[]
  onTasksChange: (tasks: AccountBackfillTask[]) => void
}) {
  const { t } = useTranslation('mail')
  const [open, setOpen] = useState(false)
  const [delayElapsed, setDelayElapsed] = useState(false)
  const [busyAccountId, setBusyAccountId] = useState<string | null>(null)
  const [controlError, setControlError] = useState<string | null>(null)

  const hasUrgent = tasks.some((task) => task.phase === 'paused' || task.phase === 'failed')

  if (tasks.length === 0) {
    if (open) setOpen(false)
    if (delayElapsed) setDelayElapsed(false)
    if (controlError !== null) setControlError(null)
  }

  useEffect(() => {
    if (tasks.length === 0 || hasUrgent || open || delayElapsed) return
    const timer = setTimeout(() => {
      setDelayElapsed(true)
    }, QUIET_SHOW_DELAY_MS)
    return () => {
      clearTimeout(timer)
    }
  }, [tasks.length, hasUrgent, open, delayElapsed])

  const control = useCallback((accountId: string, action: BackfillControlAction) => {
    setControlError(null)
    setBusyAccountId(accountId)
    void client.controlBackfill(accountId, action).then(
      (next) => {
        onTasksChange(next)
      },
      (error: unknown) => {
        setControlError(errorText(error))
      },
    ).finally(() => {
      setBusyAccountId((current) => (current === accountId ? null : current))
    })
  }, [client, onTasksChange])

  if (tasks.length === 0 || (!hasUrgent && !open && !delayElapsed)) {
    return null
  }

  const hasFailed = tasks.some((task) => task.phase === 'failed')
  const hasPaused = tasks.some((task) => task.phase === 'paused')
  const dot = hasFailed ? 'bg-destructive' : hasPaused ? 'bg-amber-500' : null
  const fraction = aggregateProgress(tasks)
  const offset = RING_CIRCUMFERENCE * (1 - fraction)
  const accountNames = new Map(accounts.map((account) => [account.id, account.displayName]))

  return (
    <TitleBarActions side="trailing" order={-3}>
      <Popover open={open} onOpenChange={setOpen}>
        <TooltipProvider>
          <Tooltip>
            <TooltipTrigger
              render={
                <PopoverTrigger
                  render={
                    <Button
                      variant="ghost"
                      size="icon-sm"
                      aria-label={t('syncTasks')}
                    />
                  }
                />
              }
            >
              <span className="relative flex items-center justify-center">
                <svg viewBox="0 0 20 20" className="size-4" aria-hidden>
                  <circle
                    cx={10}
                    cy={10}
                    r={RING_RADIUS}
                    fill="none"
                    strokeWidth={1.75}
                    className="stroke-muted-foreground/25"
                  />
                  <circle
                    data-testid="sync-tasks-ring-progress"
                    cx={10}
                    cy={10}
                    r={RING_RADIUS}
                    fill="none"
                    strokeWidth={1.75}
                    strokeLinecap="round"
                    strokeDasharray={RING_CIRCUMFERENCE}
                    strokeDashoffset={offset}
                    transform="rotate(-90 10 10)"
                    className="stroke-primary transition-[stroke-dashoffset] duration-300"
                  />
                  <path
                    d="M10 6.5v5m0 0-2-2m2 2 2-2"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth={1.6}
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                </svg>
                {dot && (
                  <span
                    aria-hidden
                    data-testid="sync-tasks-dot"
                    className={`absolute -top-0.5 -right-0.5 size-1.5 rounded-full ring-2 ring-background ${dot}`}
                  />
                )}
              </span>
            </TooltipTrigger>
            <TooltipContent side="bottom">{t('syncTasks')}</TooltipContent>
          </Tooltip>
        </TooltipProvider>
        <PopoverContent align="end" className="w-72 gap-2 p-2.5">
          <div className="flex items-center justify-between">
            <span className="text-xs font-medium text-foreground">{t('syncTasksTitle')}</span>
          </div>
          {controlError && (
            <p role="alert" className="text-[11px] text-destructive">
              {controlError}
            </p>
          )}
          <TooltipProvider>
            <div className="flex flex-col divide-y divide-border/50">
              {tasks.map((task) => {
                const accountName = accountNames.get(task.accountId) ?? task.accountId
                const title = task.activeFolderName
                  ? t('syncTaskFolder', { account: accountName, folder: task.activeFolderName })
                  : accountName
                const coveredText = task.targetDate === 'all' && task.coveredSince === 'all'
                  ? t('syncTaskCoveredAll')
                  : task.coveredSince && task.coveredSince !== 'all'
                    ? t('syncTaskCovered', { date: task.coveredSince })
                    : null
                const progressText = task.total !== null
                  ? t('syncTaskProgressCount', { fetched: task.fetched, total: task.total })
                  : t('syncTaskProgressUnknown', { fetched: task.fetched })
                const ratio = task.total !== null && task.total > 0
                  ? Math.min(100, Math.round((task.fetched / task.total) * 100))
                  : null
                const busy = busyAccountId === task.accountId
                const primaryAction: BackfillControlAction = task.phase === 'running' ? 'pause' : 'resume'
                const primaryLabel = task.phase === 'running'
                  ? t('syncTaskPause')
                  : task.phase === 'failed'
                    ? t('syncTaskRetry')
                    : t('syncTaskResume')
                const primaryIcon = task.phase === 'running'
                  ? PauseIcon
                  : task.phase === 'failed'
                    ? RefreshIcon
                    : PlayIcon
                return (
                  <div key={task.accountId} className="flex flex-col gap-1 py-1.5 first:pt-0 last:pb-0">
                    <div className="flex items-center justify-between gap-1.5">
                      <div className="flex min-w-0 flex-1 items-center gap-1.5">
                        <span className="truncate text-xs font-medium text-foreground">{title}</span>
                        {task.phase === 'paused' && (
                          <span className="shrink-0 text-[11px] text-amber-600 dark:text-amber-400">
                            {t('syncTaskPaused')}
                          </span>
                        )}
                        {task.phase === 'failed' && (
                          <span className="shrink-0 text-[11px] text-destructive">
                            {t('syncTaskFailed')}
                          </span>
                        )}
                      </div>
                      <div className="flex shrink-0 items-center gap-0.5">
                        <Tooltip>
                          <TooltipTrigger
                            render={
                              <Button
                                type="button"
                                variant="ghost"
                                size="icon-sm"
                                aria-label={primaryLabel}
                                disabled={busy}
                                onClick={() => control(task.accountId, primaryAction)}
                              />
                            }
                          >
                            <HugeiconsIcon icon={primaryIcon} className="size-3.5" />
                          </TooltipTrigger>
                          <TooltipContent side="bottom">{primaryLabel}</TooltipContent>
                        </Tooltip>
                        <Tooltip>
                          <TooltipTrigger
                            render={
                              <Button
                                type="button"
                                variant="ghost"
                                size="icon-sm"
                                aria-label={t('syncTaskCancel')}
                                disabled={busy}
                                onClick={() => control(task.accountId, 'cancel')}
                              />
                            }
                          >
                            <HugeiconsIcon icon={Cancel01Icon} className="size-3.5" />
                          </TooltipTrigger>
                          <TooltipContent side="bottom">{t('syncTaskCancel')}</TooltipContent>
                        </Tooltip>
                      </div>
                    </div>
                    {ratio !== null && (
                      <div className="h-1 w-full overflow-hidden rounded-full bg-muted">
                        <div
                          className="h-full rounded-full bg-primary transition-[width] duration-300"
                          style={{ width: `${ratio}%` }}
                        />
                      </div>
                    )}
                    <div className="flex items-center justify-between gap-2 text-[11px] leading-tight text-muted-foreground tabular-nums">
                      <span className="truncate">{progressText}</span>
                      {coveredText && <span className="shrink-0">{coveredText}</span>}
                    </div>
                    {task.phase === 'failed' && task.failure && (
                      <p className="text-[11px] leading-snug text-destructive">{errorText(task.failure)}</p>
                    )}
                  </div>
                )
              })}
            </div>
          </TooltipProvider>
          <p className="border-t border-border/50 pt-1.5 text-[11px] leading-tight text-muted-foreground">
            {t('syncTaskCancelHint')}
          </p>
        </PopoverContent>
      </Popover>
    </TitleBarActions>
  )
}
