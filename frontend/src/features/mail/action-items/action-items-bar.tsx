import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { Calendar03Icon, Cancel01Icon, CheckmarkCircle02Icon, TaskDaily01Icon } from '@hugeicons/core-free-icons'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import { Button } from '@/components/ui/button'
import { AiConsentDialog } from '@/features/ai/ai-consent-dialog'
import { AiGeneratedMark, AiMark } from '@/features/ai/ai-mark'
import { useDocumentVisible } from '@/features/ai/use-document-visible'
import { usePrefersReducedMotion } from '@/hooks/use-reduced-motion'
import type { DetectedActionItem } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'
import { cn } from '@/lib/utils'
import type { EmailActionItemsState } from './use-email-action-items'

const FOLDED_LIMIT = 2

function formatCompactTime(item: DetectedActionItem): string | null {
  if (!item.startAt) return null
  if (item.allDay || item.startAt.length <= 10) return item.startAt.slice(0, 10)
  const parsed = new Date(item.startAt)
  if (Number.isNaN(parsed.getTime())) return item.startAt
  return parsed.toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  })
}

/**
 * First-hit guidance banner shown above the body when L1 detects date/action
 * cues and the user has a usable AI model but has not enabled auto-extraction.
 */
export function ActionItemsPromptBanner({ state }: { state: EmailActionItemsState }) {
  useTranslation()
  if (!state.showPromptBanner) return null
  return (
    <div
      role="status"
      aria-label={t('mail:actionItemsPromptBanner')}
      className="flex h-9 w-full shrink-0 items-center gap-2 rounded-md bg-muted px-3 text-xs"
    >
      <AiMark className="size-3.5 shrink-0" />
      <span className="min-w-0 flex-1 truncate">{t('mail:actionItemsPromptText')}</span>
      <Button size="sm" variant="outline" className="h-6 shrink-0 text-xs" onClick={state.enableAutoFromBanner}>
        {t('mail:actionItemsPromptEnable')}
      </Button>
      <Button size="sm" variant="ghost" className="h-6 shrink-0 text-xs text-muted-foreground" onClick={state.dismissPromptForever}>
        {t('mail:actionItemsPromptNever')}
      </Button>
      <Button
        size="icon-sm"
        variant="ghost"
        className="size-6 shrink-0"
        aria-label={t('mail:actionItemsPromptClose')}
        onClick={state.closePromptForSession}
      >
        <HugeiconsIcon icon={Cancel01Icon} className="size-3.5" />
      </Button>
    </div>
  )
}

/**
 * Unified bar above the email body combining L0 (`.ics`) and L2 (AI) items.
 * Follows `ai-chat-ui.md` §4.6: AI extraction glows softly only while working
 * and fades out after completion with no persistent card border.
 */
export function ActionItemsBar({
  state,
  client,
  suppressGlow = false,
  onOpenSettings,
}: {
  state: EmailActionItemsState
  client: MailxClient
  /** True when another AI surface in the same view (e.g. summary card) owns the single glow slot. */
  suppressGlow?: boolean
  onOpenSettings: () => void
}) {
  useTranslation()
  const [expanded, setExpanded] = useState(false)
  const [consenting, setConsenting] = useState(false)
  const reduced = usePrefersReducedMotion()
  const visible = useDocumentVisible()

  const working = state.phase === 'working'
  const showConsent = state.phase === 'consent'
  const showError = state.phase === 'error'
  const showEmpty = state.showEmptyNotice
  const items = state.visibleItems

  if (!working && !showConsent && !showError && !showEmpty && items.length === 0) return null

  const displayed = expanded ? items : items.slice(0, FOLDED_LIMIT)
  const hiddenCount = items.length - displayed.length
  const glow = working && !suppressGlow && !reduced && visible

  return (
    <div className="relative shrink-0 rounded-md">
      <div
        aria-hidden
        className={cn(
          'ai-glow absolute -inset-0.5 -z-10 rounded-[inherit] opacity-0 transition-opacity',
          reduced ? 'duration-100' : 'duration-300',
          glow && 'opacity-[0.28]',
        )}
      />
      <div className="flex flex-col gap-1.5 rounded-md bg-muted/60 px-3 py-2 text-xs">
      {working && (
        <div className="flex items-center gap-2 text-muted-foreground">
          <AiMark className="size-3.5 shrink-0" />
          <span className={cn('min-w-0 flex-1', !reduced && visible && 'ai-shimmer')}>
            {t('mail:actionItemsExtracting')}
          </span>
          <Button size="sm" variant="ghost" className="h-6 text-xs" onClick={state.stop}>
            {t('mail:actionItemsStop')}
          </Button>
        </div>
      )}

      {showConsent && (
        <div className="flex flex-wrap items-center gap-2">
          <AiMark className="size-3.5 shrink-0" />
          <span className="min-w-0 flex-1">{t('mail:summaryConsent', { service: state.service })}</span>
          <Button size="sm" variant="outline" className="h-6 text-xs" onClick={() => setConsenting(true)}>
            {t('mail:summaryAllow')}
          </Button>
        </div>
      )}

      {showError && (
        <div className="flex flex-wrap items-center gap-2 text-muted-foreground">
          <AiMark className="size-3.5 shrink-0" />
          <span className="min-w-0 flex-1">
            {t('mail:actionItemsFailed')}
            {state.error ? ` · ${errorText({ code: state.error })}` : ''}
          </span>
          {state.error === 'aiNotConfigured' || state.error === 'assistantDisabled' ? (
            <Button size="sm" variant="outline" className="h-6 text-xs" onClick={onOpenSettings}>
              {t('mail:summaryOpenSettings')}
            </Button>
          ) : (
            <Button size="sm" variant="outline" className="h-6 text-xs" onClick={state.resume}>
              {t('mail:summaryRetry')}
            </Button>
          )}
        </div>
      )}

      {showEmpty && (
        <div className="flex items-center gap-2 text-muted-foreground">
          <AiGeneratedMark className="size-3.5 shrink-0" />
          <span className="min-w-0 flex-1">{t('mail:actionItemsEmpty')}</span>
          <Button size="sm" variant="outline" className="h-6 text-xs" onClick={() => state.extract(true)}>
            {t('mail:actionItemsReextract')}
          </Button>
          <Button
            size="icon-sm"
            variant="ghost"
            className="size-6 shrink-0 text-muted-foreground"
            aria-label={t('mail:actionItemsDismissEmpty')}
            onClick={state.dismissEmptyNotice}
          >
            <HugeiconsIcon icon={Cancel01Icon} className="size-3.5" />
          </Button>
        </div>
      )}

      {displayed.map((item) => {
        const timeLabel = formatCompactTime(item)
        const isAi = item.source === 'ai'
        const isAdded = item.status === 'added'
        return (
          <div key={item.id} className="flex items-center gap-2">
            {isAi ? (
              <AiGeneratedMark className="size-3.5 shrink-0" />
            ) : (
              <HugeiconsIcon
                icon={item.kind === 'event' ? Calendar03Icon : TaskDaily01Icon}
                className="size-3.5 shrink-0 text-muted-foreground"
              />
            )}
            <span className={cn('min-w-0 flex-1 truncate font-medium text-foreground', item.cancelled && 'line-through text-muted-foreground')}>
              {item.title}
            </span>
            {timeLabel && (
              <span className="shrink-0 text-muted-foreground tabular-nums">{timeLabel}</span>
            )}
            {item.cancelled ? (
              <span className="shrink-0 rounded bg-muted px-1.5 py-0.5 text-[11px] text-muted-foreground">
                {t('mail:actionItemCancelled')}
              </span>
            ) : (
              <>
                {isAdded && (
                  <span className="inline-flex shrink-0 items-center gap-1 text-[11px] text-muted-foreground">
                    <HugeiconsIcon icon={CheckmarkCircle02Icon} className="size-3" />
                    {t('mail:actionItemAdded')}
                  </span>
                )}
                <Button
                  size="sm"
                  variant="outline"
                  className="h-6 shrink-0 px-2 text-xs"
                  onClick={() => state.openItem(item)}
                >
                  {t('mail:actionItemAdd')}
                </Button>
              </>
            )}
            <Button
              size="icon-sm"
              variant="ghost"
              className="size-6 shrink-0 text-muted-foreground"
              aria-label={t('mail:actionItemDismiss')}
              onClick={() => void state.markStatus(item.id, 'dismissed')}
            >
              <HugeiconsIcon icon={Cancel01Icon} className="size-3.5" />
            </Button>
          </div>
        )
      })}

      {hiddenCount > 0 && (
        <button
          type="button"
          className="self-start text-xs text-muted-foreground hover:text-foreground"
          onClick={() => setExpanded(true)}
        >
          {t('mail:actionItemsExpand', { count: items.length })}
        </button>
      )}
      {expanded && items.length > FOLDED_LIMIT && (
        <button
          type="button"
          className="self-start text-xs text-muted-foreground hover:text-foreground"
          onClick={() => setExpanded(false)}
        >
          {t('mail:actionItemsCollapse')}
        </button>
      )}

      <AiConsentDialog
        open={consenting}
        onOpenChange={setConsenting}
        client={client}
        service={state.service}
        onGranted={() => state.resume()}
      />
      </div>
    </div>
  )
}
