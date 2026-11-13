import { Fragment, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import { cn } from '@/lib/utils'
import { SHORTCUT_GROUPS, SHORTCUTS } from './shortcuts'

/** One key cap. */
export function Kbd({ children }: { children: ReactNode }) {
  return (
    <kbd className="inline-flex h-5 min-w-5 items-center justify-center rounded-sm bg-muted px-1 font-sans text-[0.7rem] font-medium text-muted-foreground">
      {children}
    </kbd>
  )
}

/** Alternatives side by side ("J or ↓"), each a run of key caps. */
function ShortcutKeys({ keys }: { keys: string[][] }) {
  return (
    <span className="flex shrink-0 items-center gap-1.5">
      {keys.map((combo, index) => (
        <Fragment key={combo.join('+')}>
          {index > 0 && <span className="text-[0.7rem] text-muted-foreground">{t('mail:shortcutsOr')}</span>}
          <span className="flex items-center gap-0.5">{combo.map((key) => <Kbd key={key}>{key}</Kbd>)}</span>
        </Fragment>
      ))}
    </span>
  )
}

/**
 * Every shortcut by group, from the central list. With `singleKeyOff`, the
 * single-key rows are dimmed and marked off; the modifier ones stay as they are.
 */
export function ShortcutList({ singleKeyOff = false, className }: { singleKeyOff?: boolean; className?: string }) {
  useTranslation()
  return (
    <div className={cn('flex flex-col gap-5', className)}>
      {SHORTCUT_GROUPS.map((group) => (
        <section key={group.id} aria-labelledby={`shortcut-group-${group.id}`} className="flex flex-col gap-1.5">
          <h3 id={`shortcut-group-${group.id}`} className="text-xs font-medium text-muted-foreground">{t(group.labelKey)}</h3>
          <ul className="flex flex-col gap-1">
            {SHORTCUTS.filter((item) => item.group === group.id).map((item) => {
              const off = singleKeyOff && item.singleKey
              return (
                <li
                  key={item.id}
                  data-shortcut={item.id}
                  data-off={off ? '' : undefined}
                  className={cn('flex min-h-7 items-center justify-between gap-4 text-xs', off && 'opacity-50')}
                >
                  <span className="min-w-0">
                    {t(item.labelKey)}
                    {off && <span className="sr-only">{t('mail:shortcutOff')}</span>}
                  </span>
                  <ShortcutKeys keys={item.keys} />
                </li>
              )
            })}
          </ul>
        </section>
      ))}
    </div>
  )
}
