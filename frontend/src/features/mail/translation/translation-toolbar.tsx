import { useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { ArrowDown01Icon, LanguagesIcon } from '@hugeicons/core-free-icons'
import { t } from '@/i18n'
import { Button } from '@/components/ui/button'
import { IconButton } from '@/components/icon-button'
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuGroup,
  DropdownMenuItem,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
  DropdownMenuTrigger,
} from '@/components/ui/dropdown-menu'
import { Popover, PopoverContent } from '@/components/ui/popover'
import { Spinner } from '@/components/ui/spinner'
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip'
import { languageName, nativeLanguageName, TRANSLATION_TARGETS } from './languages'
import type { EmailTranslation } from './use-email-translation'

/** The "always translate" and "never suggest" items for the detected language; shared by the toolbar menu and the notice bar. */
export function DetectedLanguageItems({ translation }: { translation: EmailTranslation }) {
  useTranslation()
  const { detectedLanguage } = translation
  if (!detectedLanguage) return null
  const language = languageName(detectedLanguage)
  return <>
    <DropdownMenuCheckboxItem checked={translation.alwaysTranslating} onCheckedChange={() => translation.toggleLanguageList('always')}>
      {t('mail:translationAlwaysLanguage', { language })}
    </DropdownMenuCheckboxItem>
    <DropdownMenuCheckboxItem checked={translation.neverSuggesting} onCheckedChange={() => translation.toggleLanguageList('never')}>
      {t('mail:translationNeverLanguage', { language })}
    </DropdownMenuCheckboxItem>
  </>
}

/**
 * The reader toolbar's translation control: a button that translates or flips
 * between the original and the translation, and a small menu beside it. Without
 * a translation service the button explains what is missing instead of failing.
 */
export function TranslationToolbarGroup({ translation }: { translation: EmailTranslation }) {
  useTranslation()
  const pair = useRef<HTMLDivElement>(null)
  const [needsSetup, setNeedsSetup] = useState(false)
  const { status, translationVisible, phase, configured } = translation
  const busy = phase === 'translating'
  const hasTranslation = Boolean(status?.translation)
  // Nothing happens without a service; say so next to the button instead.
  const guarded = (run: () => void) => () => { if (configured) run(); else setNeedsSetup(true) }
  const tip = translationVisible ? t('mail:translationShowOriginal') : hasTranslation ? t('mail:translationShowTranslation') : undefined

  return (
    <div ref={pair} className="flex items-center not-first:ml-3">
      <IconButton
        label={t('mail:translateButton')}
        tip={tip}
        aria-pressed={translationVisible}
        aria-busy={busy || undefined}
        className="aria-pressed:bg-muted aria-pressed:text-foreground"
        onClick={hasTranslation ? translation.toggle : guarded(translation.toggle)}
      >
        {busy ? <Spinner aria-hidden role="presentation" /> : <HugeiconsIcon icon={LanguagesIcon} />}
      </IconButton>
      <DropdownMenu>
        <Tooltip>
          <TooltipTrigger
            render={<DropdownMenuTrigger render={<Button variant="ghost" aria-label={t('mail:translationOptions')} className="h-7 w-4 rounded-md px-0" />} />}
          >
            <HugeiconsIcon icon={ArrowDown01Icon} className="size-3 text-muted-foreground" />
          </TooltipTrigger>
          <TooltipContent>{t('mail:translationOptions')}</TooltipContent>
        </Tooltip>
        <DropdownMenuContent align="start" className="w-56">
          <DropdownMenuGroup>
            <DropdownMenuSub>
              <DropdownMenuSubTrigger>
                <span className="min-w-0 flex-1 truncate">{t('mail:translationTranslateTo')}</span>
                {translation.target && <span className="max-w-24 truncate text-muted-foreground">{languageName(translation.target)}</span>}
              </DropdownMenuSubTrigger>
              <DropdownMenuSubContent className="max-h-72 w-44">
                <DropdownMenuRadioGroup value={translation.target ?? ''} onValueChange={(value) => guarded(() => translation.translate(value))()}>
                  {TRANSLATION_TARGETS.map((code) => (
                    <DropdownMenuRadioItem key={code} value={code}>{nativeLanguageName(code)}</DropdownMenuRadioItem>
                  ))}
                </DropdownMenuRadioGroup>
              </DropdownMenuSubContent>
            </DropdownMenuSub>
            {translation.detectedLanguage && <DropdownMenuSeparator />}
            <DetectedLanguageItems translation={translation} />
          </DropdownMenuGroup>
          <DropdownMenuSeparator />
          {hasTranslation && <DropdownMenuItem disabled={busy} onClick={translation.retranslate}>{t('mail:translationRetranslate')}</DropdownMenuItem>}
          <DropdownMenuItem onClick={translation.openSettings}>{t('mail:translationSettings')}</DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <Popover open={needsSetup} onOpenChange={setNeedsSetup}>
        <PopoverContent anchor={pair} align="start">
          <p>{t('mail:translationNotSetUp')}</p>
          <Button size="sm" variant="outline" className="h-6 self-start text-xs" onClick={() => { setNeedsSetup(false); translation.openSettings() }}>
            {t('mail:translationOpenSettings')}
          </Button>
        </PopoverContent>
      </Popover>
    </div>
  )
}
