import { useId, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import { HugeiconsIcon } from '@hugeicons/react'
import { MoreHorizontalIcon } from '@hugeicons/core-free-icons'
import { IconButton } from '@/components/icon-button'
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'
import { cn } from '@/lib/utils'
import { motionProps, type EntranceMode } from '../motion'

export type ModuleMenu = {
  onHideToday: () => void
  onHideForever: () => void
}

/**
 * One block of the Now page: a small title and the content. A hideable module
 * has a "more" button at the end of the title row; it shows while the pointer
 * is over the module or focus is in it, and stays in the tab order.
 */
export function ModuleSection({ title, mode, index, menu, children }: { title: string; mode: EntranceMode; index: number; menu?: ModuleMenu; children: ReactNode }) {
  useTranslation()
  const id = useId()
  return (
    <section aria-labelledby={id} {...motionProps(mode, index, 'group/module flex flex-col gap-2')}>
      <div className="flex items-center justify-between gap-2 pr-1">
        <h2 id={id} className="px-3 text-xs font-medium text-muted-foreground">{title}</h2>
        {menu && (
          <DropdownMenu>
            <DropdownMenuTrigger
              render={(
                <IconButton
                  label={t('now:moduleMore', { module: title })}
                  size="icon-sm"
                  className={cn('-my-1 text-muted-foreground opacity-0 transition-opacity duration-150 group-hover/module:opacity-100 group-focus-within/module:opacity-100 data-popup-open:opacity-100 motion-reduce:transition-none')}
                >
                  <HugeiconsIcon icon={MoreHorizontalIcon} />
                </IconButton>
              )}
            />
            <DropdownMenuContent align="end" className="w-44">
              <DropdownMenuItem onClick={menu.onHideToday}>{t('now:hideToday')}</DropdownMenuItem>
              <DropdownMenuItem onClick={menu.onHideForever}>{t('now:hideForever')}</DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        )}
      </div>
      {children}
    </section>
  )
}
