import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { MoreHorizontalIcon } from '@hugeicons/core-free-icons'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import { Button } from '@/components/ui/button'
import { DropdownMenu, DropdownMenuContent, DropdownMenuTrigger } from '@/components/ui/dropdown-menu'
import { Spinner } from '@/components/ui/spinner'
import { clientFailureCode } from '@/lib/mailx-client'
import { languageName, TRANSLATION_PROVIDER_NAMES } from './languages'
import { DetectedLanguageItems } from './translation-toolbar'
import type { EmailTranslation } from './use-email-translation'

const action = 'h-6 shrink-0 text-xs'
const KEYLESS_FAILURES: string[] = ['translationUnavailable', 'translationRateLimited', 'translationNetwork', 'translationTimeout']

/**
 * One quiet line above the message that follows the translation: an offer, the
 * progress, what is showing, or why it failed. It has the same height and look
 * as the remote-images notice, so nothing moves when its content changes.
 */
export function TranslationBar({ translation }: { translation: EmailTranslation }) {
  useTranslation()
  const { status, phase, error } = translation
  const translated = status?.translation ?? null
  const target = translation.target ? languageName(translation.target) : ''
  const shell = (children: React.ReactNode) => (
    <div role="status" aria-label={t('mail:translationBarLabel')} className="flex h-9 w-full shrink-0 items-center gap-3 rounded-md bg-muted px-3 text-xs">
      {children}
    </div>
  )
  const text = (value: string, extra?: React.ReactNode) => (
    <span className="min-w-0 flex-1 truncate" title={value}>{value}{extra}</span>
  )
  const close = (onClick: () => void) => (
    <Button size="icon-sm" variant="ghost" className="size-6 shrink-0" aria-label={t('mail:translationDismiss')} onClick={onClick}>×</Button>
  )

  if (phase === 'translating') {
    return shell(<>
      <Spinner aria-hidden role="presentation" className="size-3.5 shrink-0 text-muted-foreground" />
      {text(t('mail:translationTranslating'))}
      <Button size="sm" variant="outline" className={action} onClick={translation.cancel}>{t('mail:cancel')}</Button>
    </>)
  }
  if (phase === 'error') {
    const message = errorText(error)
    const code = clientFailureCode(error)
    // A keyless channel that refuses, is limited or cannot be reached has no key to fix: offer another service next to the retry.
    const channelFailed = translation.keyless && code !== null && KEYLESS_FAILURES.includes(code)
    return shell(<>
      {text(message)}
      {code === 'translationNotConfigured'
        ? <Button size="sm" variant="outline" className={action} onClick={translation.openSettings}>{t('mail:translationOpenSettings')}</Button>
        : <Button size="sm" variant="outline" className={action} onClick={() => translation.translate(translation.target ?? undefined)}>{t('mail:retry')}</Button>}
      {channelFailed && <Button size="sm" variant="ghost" className={`${action} text-muted-foreground`} onClick={translation.openSettings}>{t('mail:translationUseAnotherService')}</Button>}
      {close(translation.dismissError)}
    </>)
  }
  if (translation.translationVisible && translated) {
    const from = translated.sourceLanguage ? t('mail:translationTranslatedFrom', { language: languageName(translated.sourceLanguage) }) : t('mail:translationTranslated')
    const partial = translated.partial ? t('mail:translationPartial') : ''
    const service = TRANSLATION_PROVIDER_NAMES[translated.provider]
    return shell(<>
      <span className="min-w-0 flex-1 truncate" title={[from, service, partial].filter(Boolean).join(' · ')}>
        {from}
        <span className="text-muted-foreground"> · {service}{partial && ` · ${partial}`}</span>
      </span>
      <Button size="sm" variant="outline" className={action} onClick={translation.showOriginal}>{t('mail:translationShowOriginal')}</Button>
    </>)
  }
  if (translated && translation.viewed) {
    return shell(<>
      {text(t('mail:translationShowingOriginal'))}
      <Button size="sm" variant="outline" className={action} onClick={translation.showTranslation}>{t('mail:translationShowTranslation')}</Button>
    </>)
  }
  if (status?.suggested && !translation.dismissed) {
    return shell(<>
      {text(t('mail:translationMessageIn', { language: status.detectedLanguage ? languageName(status.detectedLanguage) : '' }))}
      {translated
        ? <Button size="sm" variant="outline" className={action} onClick={translation.showTranslation}>{t('mail:translationShowTranslation')}</Button>
        : <Button size="sm" variant="outline" className={action} onClick={() => translation.translate()}>{t('mail:translationTranslateToTarget', { target })}</Button>}
      <DropdownMenu>
        <DropdownMenuTrigger render={<Button size="icon-sm" variant="ghost" className="size-6 shrink-0" aria-label={t('mail:more')} />}>
          <HugeiconsIcon icon={MoreHorizontalIcon} />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-56"><DetectedLanguageItems translation={translation} /></DropdownMenuContent>
      </DropdownMenu>
      {close(translation.dismiss)}
    </>)
  }
  return null
}
