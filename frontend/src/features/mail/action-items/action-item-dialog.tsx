import { useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { MoreHorizontalIcon } from '@hugeicons/core-free-icons'
import { t } from '@/i18n'
import { errorDetail } from '@/i18n/errors'
import { ErrorNotice } from '@/components/error-notice'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'
import { Input } from '@/components/ui/input'
import { Textarea } from '@/components/ui/textarea'
import { toast } from '@/components/ui/toast'
import type { ActionItemExportInput, ActionItemKind, DetectedActionItem, EmailSummary } from '@/generated/accounts'
import { clientFailure, type ClientFailure, type MailxClient } from '@/lib/mailx-client'
import { cn } from '@/lib/utils'

function toDatetimeLocalValue(iso: string | null, allDay: boolean): string {
  if (!iso) return ''
  if (allDay || iso.length <= 10) return iso.slice(0, 10)
  const d = new Date(iso)
  if (Number.isNaN(d.getTime())) return ''
  const pad = (n: number) => String(n).padStart(2, '0')
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`
}

function fromDatetimeLocalValue(value: string, allDay: boolean): string | null {
  const trimmed = value.trim()
  if (!trimmed) return null
  if (allDay) return trimmed.slice(0, 10)
  const d = new Date(trimmed)
  if (Number.isNaN(d.getTime())) return null
  return d.toISOString()
}

function defaultNotes(item: DetectedActionItem, summary: EmailSummary): string {
  const sender = summary.from.name ? `${summary.from.name} <${summary.from.address}>` : summary.from.address
  const sourceLine = t('mail:actionItemSourceNote', { subject: summary.subject || t('mail:noSubject'), sender })
  if (item.notes && item.notes.trim()) return `${item.notes.trim()}\n\n${sourceLine}`
  return sourceLine
}

export function ActionItemDialog({
  client,
  emailSummary,
  item,
  onClose,
  onAdded,
}: {
  client: MailxClient
  emailSummary: EmailSummary
  item: DetectedActionItem | null
  onClose: () => void
  onAdded: (itemId: string) => void
}) {
  if (!item) return null
  return (
    <ActionItemDialogBody
      key={`${emailSummary.id}:${item.id}`}
      client={client}
      emailSummary={emailSummary}
      item={item}
      onClose={onClose}
      onAdded={onAdded}
    />
  )
}

function ActionItemDialogBody({
  client,
  emailSummary,
  item,
  onClose,
  onAdded,
}: {
  client: MailxClient
  emailSummary: EmailSummary
  item: DetectedActionItem
  onClose: () => void
  onAdded: (itemId: string) => void
}) {
  useTranslation()
  const [kind, setKind] = useState<ActionItemKind>(item.kind)
  const [title, setTitle] = useState(item.title)
  const [allDay, setAllDay] = useState(item.allDay)
  const [startValue, setStartValue] = useState(() => toDatetimeLocalValue(item.startAt, item.allDay))
  const [endValue, setEndValue] = useState(() => toDatetimeLocalValue(item.endAt, item.allDay))
  const [location, setLocation] = useState(item.location ?? '')
  const [url, setUrl] = useState(item.url ?? '')
  const [notes, setNotes] = useState(() => defaultNotes(item, emailSummary))
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<ClientFailure | null>(null)

  const [openedAt] = useState(() => Date.now())
  const isPast = useMemo(() => {
    const iso = fromDatetimeLocalValue(startValue, allDay)
    if (!iso) return false
    const ts = allDay ? new Date(`${iso}T23:59:59`).getTime() : new Date(iso).getTime()
    return !Number.isNaN(ts) && ts < openedAt
  }, [startValue, allDay, openedAt])

  const buildPayload = (): ActionItemExportInput => ({
    emailId: emailSummary.id,
    itemId: item.id,
    kind,
    title: title.trim(),
    startAt: fromDatetimeLocalValue(startValue, allDay),
    endAt: kind === 'event' ? fromDatetimeLocalValue(endValue, allDay) : null,
    allDay,
    rrule: kind === 'event' ? item.rrule : null,
    location: kind === 'event' && location.trim() ? location.trim() : null,
    url: url.trim() || null,
    notes: notes.trim() || null,
  })

  const runAction = async (fn: () => Promise<void>) => {
    setBusy(true)
    setError(null)
    try {
      await fn()
    } catch (cause) {
      setError(clientFailure(cause))
    } finally {
      setBusy(false)
    }
  }

  const openCalendar = () => runAction(async () => {
    const outcome = await client.openActionItemCalendar(buildPayload())
    if (outcome === 'saved') {
      onAdded(item.id)
      toast.success(t('mail:actionItemCalendarOpened'))
      onClose()
    }
  })

  const saveIcs = () => runAction(async () => {
    const outcome = await client.saveActionItemIcs(buildPayload())
    if (outcome === 'saved') {
      onAdded(item.id)
      toast.success(t('mail:actionItemIcsSaved'))
      onClose()
    }
  })

  const copyTask = () => runAction(async () => {
    await client.copyActionItemText(buildPayload())
    onAdded(item.id)
    toast.success(t('mail:actionItemTaskCopied'))
    onClose()
  })

  const canSubmitEvent = title.trim().length > 0 && startValue.trim().length > 0
  const canSubmitTask = title.trim().length > 0

  return (
    <Dialog open onOpenChange={(open) => { if (!open && !busy) onClose() }}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{t('mail:actionItemDialogTitle')}</DialogTitle>
        </DialogHeader>

        <div className="flex flex-col gap-3 text-sm">
          <div role="tablist" className="grid grid-cols-2 gap-1 rounded-md bg-muted p-1">
            {(['event', 'task'] as const).map((tab) => (
              <button
                key={tab}
                type="button"
                role="tab"
                aria-selected={kind === tab}
                onClick={() => setKind(tab)}
                className={cn(
                  'rounded-sm px-2.5 py-1 text-xs font-medium transition-colors',
                  kind === tab ? 'bg-background text-foreground shadow-xs' : 'text-muted-foreground hover:text-foreground',
                )}
              >
                {tab === 'event' ? t('mail:actionItemKindEvent') : t('mail:actionItemKindTask')}
              </button>
            ))}
          </div>

          <div className="flex flex-col gap-1">
            <label htmlFor="action-item-title" className="text-xs text-muted-foreground">
              {t('mail:actionItemFieldTitle')}
            </label>
            <Input
              id="action-item-title"
              value={title}
              onChange={(e) => setTitle(e.target.value)}
              disabled={busy}
            />
          </div>

          <div className="flex items-center justify-between gap-2">
            <label htmlFor="action-item-start" className="text-xs text-muted-foreground">
              {kind === 'event' ? t('mail:actionItemFieldStart') : t('mail:actionItemFieldDue')}
            </label>
            <label className="flex cursor-pointer items-center gap-1.5 text-xs text-muted-foreground">
              <Checkbox
                checked={allDay}
                onCheckedChange={(checked) => {
                  const next = checked === true
                  setAllDay(next)
                  setStartValue((cur) => (next ? cur.slice(0, 10) : cur && cur.length === 10 ? `${cur}T09:00` : cur))
                  setEndValue((cur) => (next ? cur.slice(0, 10) : cur && cur.length === 10 ? `${cur}T10:00` : cur))
                }}
                disabled={busy}
              />
              {t('mail:actionItemAllDay')}
            </label>
          </div>
          <Input
            id="action-item-start"
            type={allDay ? 'date' : 'datetime-local'}
            value={startValue}
            onChange={(e) => setStartValue(e.target.value)}
            disabled={busy}
          />
          {isPast && (
            <p className="text-xs text-amber-600 dark:text-amber-400">{t('mail:actionItemPastHint')}</p>
          )}

          {kind === 'event' && (
            <>
              <div className="flex flex-col gap-1">
                <label htmlFor="action-item-end" className="text-xs text-muted-foreground">
                  {t('mail:actionItemFieldEnd')}
                </label>
                <Input
                  id="action-item-end"
                  type={allDay ? 'date' : 'datetime-local'}
                  value={endValue}
                  onChange={(e) => setEndValue(e.target.value)}
                  disabled={busy}
                />
              </div>

              <div className="flex flex-col gap-1">
                <label htmlFor="action-item-location" className="text-xs text-muted-foreground">
                  {t('mail:actionItemFieldLocation')}
                </label>
                <Input
                  id="action-item-location"
                  value={location}
                  onChange={(e) => setLocation(e.target.value)}
                  disabled={busy}
                />
              </div>
            </>
          )}

          <div className="flex flex-col gap-1">
            <label htmlFor="action-item-url" className="text-xs text-muted-foreground">
              {t('mail:actionItemFieldUrl')}
            </label>
            <Input
              id="action-item-url"
              type="url"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              disabled={busy}
              placeholder="https://"
            />
          </div>

          <div className="flex flex-col gap-1">
            <label htmlFor="action-item-notes" className="text-xs text-muted-foreground">
              {t('mail:actionItemFieldNotes')}
            </label>
            <Textarea
              id="action-item-notes"
              rows={3}
              value={notes}
              onChange={(e) => setNotes(e.target.value)}
              disabled={busy}
            />
          </div>

          {item.matchedText && (
            <div className="rounded-md bg-muted/60 px-2.5 py-1.5 text-xs text-muted-foreground">
              <span className="font-medium">{t('mail:actionItemMatchedExcerpt')}</span>
              <span>{item.matchedText}</span>
            </div>
          )}

          {error && <ErrorNotice message={t('mail:actionItemActionFailed')} detail={errorDetail(error)} />}
        </div>

        <DialogFooter className="gap-2">
          <DropdownMenu>
            <DropdownMenuTrigger
              render={
                <Button
                  type="button"
                  size="icon"
                  variant="ghost"
                  className="mr-auto text-muted-foreground"
                  aria-label={t('mail:more')}
                  disabled={busy || (kind === 'event' ? !canSubmitEvent : !canSubmitTask)}
                />
              }
            >
              <HugeiconsIcon icon={MoreHorizontalIcon} className="size-4" />
            </DropdownMenuTrigger>
            <DropdownMenuContent align="start">
              <DropdownMenuItem onClick={() => void saveIcs()}>
                {kind === 'event' ? t('mail:actionItemSaveIcs') : t('mail:actionItemExportVtodo')}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
          {kind === 'event' ? (
            <Button disabled={busy || !canSubmitEvent} onClick={() => void openCalendar()}>
              {t('mail:actionItemAddToCalendar')}
            </Button>
          ) : (
            <Button disabled={busy || !canSubmitTask} onClick={() => void copyTask()}>
              {t('mail:actionItemCopyTask')}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
