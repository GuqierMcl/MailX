import { useEffect, useId, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { DragDropProvider } from '@dnd-kit/react'
import { isSortable, useSortable } from '@dnd-kit/react/sortable'
import { Accessibility } from '@dnd-kit/dom'
import { OptimisticSortingPlugin } from '@dnd-kit/dom/sortable'
import { move } from '@dnd-kit/helpers'
import { HugeiconsIcon } from '@hugeicons/react'
import { ArrowDown01Icon, ArrowUp01Icon, DragDropVerticalIcon } from '@hugeicons/core-free-icons'
import { Radio } from '@base-ui/react/radio'
import { RadioGroup } from '@base-ui/react/radio-group'
import { Button } from '@/components/ui/button'
import { Switch } from '@/components/ui/switch'
import { usePrefersReducedMotion } from '@/hooks/use-reduced-motion'
import type { NowArrangement, NowSettings } from '@/generated/accounts'
import { t } from '@/i18n'
import { cn } from '@/lib/utils'
import type { MailxClient } from '@/lib/mailx-client'
import { fixedOrder } from '@/features/now/modules/arrange'
import { DEFAULT_NOW } from '@/features/now/modules/defaults'
import { HIDEABLE_MODULES, moduleById } from '@/features/now/modules/registry'
import type { NowModuleId } from '@/features/now/modules/types'
import { SettingsGroup, SettingsRow, SettingsSaveError } from './settings-layout'
import { useUpdateNow } from './settings-queries'

const controlledSorting: NonNullable<Parameters<typeof useSortable>[0]['plugins']> =
  (defaults) => defaults.filter((plugin) => plugin !== OptimisticSortingPlugin)

const title = (id: string) => moduleById(id as NowModuleId).title()

/** Settings → Appearance → Now: how the page arranges its modules and which ones it shows. */
export function NowGroup({ client, now }: { client: MailxClient; now: NowSettings }) {
  useTranslation()
  const update = useUpdateNow(client)
  const fixed = now.arrangement === 'fixed'
  const ids = HIDEABLE_MODULES.map((module) => module.id)
  const listed = fixed ? fixedOrder(ids, now.order) : ids
  const isDefault = now.arrangement === DEFAULT_NOW.arrangement && now.order.length === 0 && now.hidden.length === 0
  const nameId = useId()

  const show = (id: string, shown: boolean) => {
    const hidden = now.hidden.filter((other) => other !== id)
    update.mutate({ hidden: shown ? hidden : [...hidden, id] })
  }
  /** The new order of the listed modules; ids this version does not know keep their place after them. */
  const reorder = (next: string[]) => update.mutate({ order: [...next, ...now.order.filter((id) => !next.includes(id))] })

  return (
    <SettingsGroup title={t('settings:nowGroup')}>
      <SettingsRow label={t('settings:nowArrangement')} description={t('settings:nowArrangementDescription')}>
        <ArrangementChoice value={now.arrangement} onChange={(arrangement) => update.mutate({ arrangement })} />
      </SettingsRow>
      <div role="group" aria-labelledby={nameId} className="flex flex-col gap-1">
        <p id={nameId} className="text-sm font-medium">{t('settings:nowModules')}</p>
        {fixed && <p className="text-xs text-muted-foreground">{t('settings:nowModuleOrderHint')}</p>}
        {fixed
          ? <SortableModules ids={listed} hidden={now.hidden} onShow={show} onReorder={reorder} />
          : (
            <ul className="flex flex-col gap-0.5">
              {listed.map((id) => (
                <li key={id} className="flex items-center gap-2 rounded-lg px-2 py-1.5">
                  <ModuleSwitch id={id} hidden={now.hidden} onShow={show} />
                </li>
              ))}
            </ul>
          )}
      </div>
      <div>
        <Button variant="outline" aria-label={t('settings:nowResetLabel')} disabled={isDefault} onClick={() => update.mutate(DEFAULT_NOW)}>{t('settings:nowReset')}</Button>
      </div>
      <SettingsSaveError update={update} message={t('settings:saveFailed')} />
    </SettingsGroup>
  )
}

function ArrangementChoice({ value, onChange }: { value: NowArrangement; onChange: (arrangement: NowArrangement) => void }) {
  useTranslation()
  const options: { value: NowArrangement; label: string }[] = [
    { value: 'auto', label: t('settings:nowAuto') },
    { value: 'fixed', label: t('settings:nowFixed') },
  ]
  return (
    <RadioGroup aria-label={t('settings:nowArrangement')} value={value} onValueChange={(next) => onChange(next as NowArrangement)}
      className="inline-flex rounded-lg bg-muted p-0.5">
      {options.map((option) => (
        <Radio.Root key={option.value} value={option.value}
          className="inline-flex h-7 items-center rounded-md px-2.5 text-xs font-medium text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 data-checked:bg-background data-checked:text-foreground data-checked:shadow-sm">
          {option.label}
        </Radio.Root>
      ))}
    </RadioGroup>
  )
}

/** The name and its switch; the switch is named by the module, not by the row. */
function ModuleSwitch({ id, hidden, onShow }: { id: string; hidden: string[]; onShow: (id: string, shown: boolean) => void }) {
  const switchId = `now-module-${id}`
  return (
    <>
      <label htmlFor={switchId} className="min-w-0 flex-1 truncate text-sm">{title(id)}</label>
      <Switch id={switchId} checked={!hidden.includes(id)} onCheckedChange={(shown) => onShow(id, shown)} />
    </>
  )
}

/** Drag, or the arrows, to reorder; the library handles pointer and keyboard input and the sorting feedback. */
function SortableModules({ ids, hidden, onShow, onReorder }: { ids: string[]; hidden: string[]; onShow: (id: string, shown: boolean) => void; onReorder: (ids: string[]) => void }) {
  useTranslation()
  const reducedMotion = usePrefersReducedMotion()
  const startedWith = useRef<string[]>([])
  const [preview, setPreview] = useState<string[] | null>(null)
  const displayed = preview ?? ids
  const instructionsId = useId()
  const latest = useRef(ids)
  useEffect(() => { latest.current = ids }, [ids])
  const name = (id: string) => title(id)
  const accessibility = useMemo<NonNullable<ConstructorParameters<typeof Accessibility>[1]>>(() => ({
    screenReaderInstructions: { draggable: t('settings:nowReorderInstructions') },
    announcements: {
      dragstart: ({ operation: { source } }) => source ? t('settings:nowReorderStarted', {
        module: name(String(source.id)), position: isSortable(source) ? source.initialIndex + 1 : 1, count: latest.current.length,
      }) : undefined,
      dragover: ({ operation: { source } }) => source && isSortable(source) ? t('settings:nowReorderMoved', {
        module: name(String(source.id)), position: source.index + 1, count: latest.current.length,
      }) : undefined,
      dragend: ({ operation: { source }, canceled }) => source ? t(canceled ? 'settings:nowReorderCanceled' : 'settings:nowReorderFinished', {
        module: name(String(source.id)), position: isSortable(source) ? source.index + 1 : 1, count: latest.current.length,
      }) : undefined,
    },
  }), [])

  const moveBy = (id: string, offset: number) => {
    const from = ids.indexOf(id)
    const to = from + offset
    if (from < 0 || to < 0 || to >= ids.length) return
    const next = [...ids]
    next.splice(to, 0, next.splice(from, 1)[0])
    onReorder(next)
  }

  return (
    <DragDropProvider
      plugins={(defaults) => defaults.map((plugin) => plugin === Accessibility ? {
        plugin: Accessibility,
        options: accessibility,
      } : plugin)}
      onDragStart={() => { startedWith.current = ids }}
      onDragOver={(event) => setPreview((current) => move(current ?? ids, event))}
      onDragEnd={(event) => {
        // A change of the list from outside (another window) invalidates the drag's indices.
        if (event.canceled || startedWith.current.length !== ids.length || startedWith.current.some((id, index) => id !== ids[index])) {
          setPreview(null)
          return
        }
        const next = preview ?? ids
        if (next.some((id, index) => id !== ids[index])) onReorder(next)
        setPreview(null)
      }}
    >
      <span id={instructionsId} className="sr-only">{t('settings:nowReorderInstructions')}</span>
      <ul className="flex flex-col gap-0.5">
        {displayed.map((id, index) => (
          <SortableRow key={id} id={id} index={index} count={displayed.length} hidden={hidden} instructionsId={instructionsId} reducedMotion={reducedMotion}
            onShow={onShow} onMove={moveBy} />
        ))}
      </ul>
    </DragDropProvider>
  )
}

function SortableRow({ id, index, count, hidden, instructionsId, reducedMotion, onShow, onMove }: {
  id: string; index: number; count: number; hidden: string[]; instructionsId: string; reducedMotion: boolean
  onShow: (id: string, shown: boolean) => void; onMove: (id: string, offset: number) => void
}) {
  const { ref, handleRef, isDragging } = useSortable({ id, index, transition: reducedMotion ? null : undefined, plugins: controlledSorting })
  const module = title(id)
  return (
    <li ref={ref} data-dragging={isDragging} className={cn('flex items-center gap-1 rounded-lg bg-background px-1 py-1 data-[dragging=true]:z-10 data-[dragging=true]:bg-muted data-[dragging=true]:shadow-md')}>
      <button ref={handleRef} type="button" aria-describedby={instructionsId}
        aria-label={t('settings:nowReorderModule', { module })} aria-roledescription={t('settings:nowReorderRoleDescription')}
        className="flex size-7 shrink-0 touch-none cursor-grab items-center justify-center rounded-md text-muted-foreground outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring/50 active:cursor-grabbing">
        <HugeiconsIcon icon={DragDropVerticalIcon} className="size-4" aria-hidden />
      </button>
      <ModuleSwitch id={id} hidden={hidden} onShow={onShow} />
      <Button variant="ghost" size="icon-sm" aria-label={t('settings:nowMoveUp', { module })} disabled={index === 0} onClick={() => onMove(id, -1)}>
        <HugeiconsIcon icon={ArrowUp01Icon} />
      </Button>
      <Button variant="ghost" size="icon-sm" aria-label={t('settings:nowMoveDown', { module })} disabled={index === count - 1} onClick={() => onMove(id, 1)}>
        <HugeiconsIcon icon={ArrowDown01Icon} />
      </Button>
    </li>
  )
}
