import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { useEffect } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import { Cancel01Icon, Search01Icon } from '@hugeicons/core-free-icons'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'

type MailSearchProps = {
  /** The search applied to the list; empty when the list is not filtered. */
  value: string
  /** What is searched, e.g. "工作邮箱 · 收件箱". */
  scope: string
  onOpen: () => void
  onClear: () => void
}

const SHORTCUT = /Mac|iPhone|iPad/.test(globalThis.navigator?.platform ?? '') ? '⌘K' : 'Ctrl K'

/** The title-bar search entry: it opens the command palette and shows the applied search. */
export function MailSearch({ value, scope, onOpen, onClear }: MailSearchProps) {
  useTranslation()
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if ((event.key === 'k' || event.key === 'f') && (event.metaKey || event.ctrlKey) && !event.altKey && !event.shiftKey) {
        event.preventDefault()
        onOpen()
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [onOpen])

  return (
    <div className="relative w-full max-w-md">
      <button
        type="button"
        aria-label={t('mail:searchMail')}
        title={t('mail:searchTooltip')}
        aria-haspopup="dialog"
        onClick={onOpen}
        className={cn(
          'flex h-7 w-full items-center gap-2 rounded-md border border-input bg-background/60 pr-7 pl-2 text-left text-xs outline-hidden transition-colors hover:bg-background focus-visible:ring-2 focus-visible:ring-ring',
          value ? 'text-foreground' : 'text-muted-foreground',
        )}
      >
        <HugeiconsIcon icon={Search01Icon} className="size-3.5 shrink-0" />
        <span className="min-w-0 flex-1 truncate">{value || t('mail:searchIn', { scope })}</span>
        {!value && <kbd className="shrink-0 font-sans text-[0.65rem] text-muted-foreground/70">{SHORTCUT}</kbd>}
      </button>
      {value && (
        <Button
          variant="ghost"
          size="icon-xs"
          aria-label={t('mail:clearSearch')}
          className="absolute top-1/2 right-1 size-5 -translate-y-1/2"
          onClick={onClear}
        >
          <HugeiconsIcon icon={Cancel01Icon} strokeWidth={2} />
        </Button>
      )}
    </div>
  )
}
