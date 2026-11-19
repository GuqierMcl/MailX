import { useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { ArrowDown01Icon, ArrowUp01Icon, Cancel01Icon, Copy01Icon, RefreshIcon } from '@hugeicons/core-free-icons'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { IconButton } from '@/components/icon-button'
import { Popover, PopoverContent } from '@/components/ui/popover'
import { Select, SelectContent, SelectItem, SelectSeparator, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Spinner } from '@/components/ui/spinner'
import { toast } from '@/components/ui/toast'
import { AiConsentDialog } from '@/features/ai/ai-consent-dialog'
import { AiGeneratedMark, AiMark } from '@/features/ai/ai-mark'
import { useDocumentVisible } from '@/features/ai/use-document-visible'
import { usePrefersReducedMotion } from '@/hooks/use-reduced-motion'
import { Markdown } from '@/features/ai/markdown-view'
import type { SummaryLanguage, SummaryStyle } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'
import { cn } from '@/lib/utils'
import { nativeLanguageName, TRANSLATION_TARGETS } from '../translation/languages'
import type { EmailSummaryState } from './use-email-summary'

const STYLES: { id: SummaryStyle; label: () => string; lines: number[] }[] = [
  { id: 'brief', label: () => t('mail:summaryStyleBrief'), lines: [14, 10] },
  { id: 'detailed', label: () => t('mail:summaryStyleDetailed'), lines: [14, 14, 14, 9] },
  { id: 'points', label: () => t('mail:summaryStylePoints'), lines: [-11, -11, -11] },
]

/** A small picture of the style: lines of text, or bullets for points. */
function StyleGlyph({ lines }: { lines: number[] }) {
  return (
    <svg aria-hidden viewBox="0 0 20 16" className="h-4 w-5">
      {lines.map((length, index) => {
        const y = 2 + index * (lines.length > 3 ? 4 : 5)
        return length < 0
          ? <g key={index}><circle cx="2.5" cy={y} r="1.2" fill="currentColor" /><rect x="6" y={y - 0.75} width={-length} height="1.5" rx="0.75" fill="currentColor" /></g>
          : <rect key={index} x="2" y={y - 0.75} width={length} height="1.5" rx="0.75" fill="currentColor" />
      })}
    </svg>
  )
}

const INTERFACE = 'interface'
const ORIGINAL = 'original'
const languageValue = (language: SummaryLanguage) => language.mode === 'fixed' ? language.code : language.mode
const languageFrom = (value: string): SummaryLanguage =>
  value === INTERFACE ? { mode: 'interface' } : value === ORIGINAL ? { mode: 'original' } : { mode: 'fixed', code: value }

/**
 * The reader toolbar's AI button: a small panel to pick the style and language, mark the sender as always
 * summarized, and make the summary. "Summarize" has the focus, so Enter makes it.
 */
export function SummaryToolbarButton({ summary, senderName }: { summary: EmailSummaryState; senderName: string }) {
  useTranslation()
  const anchor = useRef<HTMLDivElement>(null)
  const generateButton = useRef<HTMLButtonElement>(null)
  const [open, setOpen] = useState(false)
  const [style, setStyle] = useState(summary.preferredStyle)
  const [language, setLanguage] = useState(languageValue(summary.preferredLanguage))
  const busy = summary.phase === 'working'
  const languages: Record<string, string> = {
    [INTERFACE]: t('mail:summaryLanguageInterface'),
    [ORIGINAL]: t('mail:summaryLanguageOriginal'),
    ...Object.fromEntries(TRANSLATION_TARGETS.map((code) => [code, nativeLanguageName(code)])),
  }
  const show = (next: boolean) => {
    if (next) {
      // The panel opens on the saved choices.
      setStyle(summary.preferredStyle)
      setLanguage(languageValue(summary.preferredLanguage))
    }
    setOpen(next)
  }
  const make = () => {
    setOpen(false)
    summary.generate(style, languageFrom(language))
  }
  return (
    <div ref={anchor} className="flex items-center not-first:ml-3">
      <IconButton
        label={t('mail:summaryButton')}
        aria-expanded={open}
        aria-busy={busy || undefined}
        className="aria-expanded:bg-muted"
        onClick={() => show(!open)}
      >
        {busy ? <Spinner aria-hidden role="presentation" /> : <AiMark className="size-4" />}
      </IconButton>
      <Popover open={open} onOpenChange={show}>
        <PopoverContent anchor={anchor} align="start" initialFocus={generateButton} className="w-72 gap-3 text-sm" aria-label={t('mail:summaryPanelTitle')}>
          <p className="font-medium">{t('mail:summaryPanelTitle')}</p>
          <div role="radiogroup" aria-label={t('mail:summaryStyle')} className="grid grid-cols-3 gap-1.5">
            {STYLES.map((item) => (
              <button
                key={item.id}
                type="button"
                role="radio"
                aria-checked={style === item.id}
                onClick={() => setStyle(item.id)}
                className={cn(
                  'flex flex-col items-center gap-1.5 rounded-md px-2 py-2 text-xs text-muted-foreground outline-none transition-colors hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring',
                  style === item.id && 'bg-muted text-foreground',
                )}
              >
                <StyleGlyph lines={item.lines} />
                {item.label()}
              </button>
            ))}
          </div>
          <div className="flex items-center justify-between gap-3">
            <label htmlFor="summary-language" className="text-muted-foreground">{t('mail:summaryLanguage')}</label>
            <Select value={language} items={languages} onValueChange={(value: string | null) => value && setLanguage(value)}>
              <SelectTrigger id="summary-language" size="sm" className="w-40"><SelectValue /></SelectTrigger>
              <SelectContent className="max-h-72">
                <SelectItem value={INTERFACE}>{languages[INTERFACE]}</SelectItem>
                <SelectItem value={ORIGINAL}>{languages[ORIGINAL]}</SelectItem>
                <SelectSeparator />
                {TRANSLATION_TARGETS.map((code) => <SelectItem key={code} value={code}>{languages[code]}</SelectItem>)}
              </SelectContent>
            </Select>
          </div>
          {summary.senderAddress && (
            <label className="flex items-start gap-2 text-muted-foreground">
              <Checkbox className="mt-0.5" checked={summary.alwaysForSender} onCheckedChange={(checked) => summary.setAlwaysForSender(checked === true)} />
              <span className="min-w-0 break-words">{t('mail:summaryAlwaysSender', { sender: senderName })}</span>
            </label>
          )}
          <Button ref={generateButton} onClick={make}>{t('mail:summaryGenerate')}</Button>
        </PopoverContent>
      </Popover>
    </div>
  )
}

/**
 * The summary above the body (ai-chat-ui.md 5.2): the same muted surface as the translation bar, folded
 * to its first line when collapsed. Written text streams in; errors and the consent ask stay in the card.
 */
export function SummaryCard({ summary, client, onAsk, onOpenSettings }: {
  summary: EmailSummaryState
  client: MailxClient
  onAsk?: () => void
  onOpenSettings: () => void
}) {
  useTranslation()
  const [consenting, setConsenting] = useState(false)
  const reduced = usePrefersReducedMotion()
  const visible = useDocumentVisible()
  if (!summary.shown) return null
  const { phase, collapsed } = summary
  const working = phase === 'working'
  const styleName = STYLES.find((item) => item.id === summary.style)?.label()
  const copy = () => {
    void navigator.clipboard.writeText(summary.text).then(
      () => toast.success(t('mail:summaryCopied')),
      () => undefined,
    )
  }
  const openLink = (url: string) => { void client.openExternalLink(url).catch(() => undefined) }
  const action = 'size-6 text-muted-foreground'

  let body: React.ReactNode
  if (phase === 'consent') {
    body = (
      <div className="flex flex-wrap items-center gap-2">
        <span className="min-w-0 flex-1">{t('mail:summaryConsent', { service: summary.service })}</span>
        <Button size="sm" variant="outline" className="h-6 text-xs" onClick={() => setConsenting(true)}>{t('mail:summaryAllow')}</Button>
      </div>
    )
  } else if (phase === 'error') {
    const notConfigured = summary.error === 'aiNotConfigured' || summary.error === 'assistantDisabled'
    body = (
      <div className="flex flex-wrap items-center gap-2">
        <span className="min-w-0 flex-1">{t('mail:summaryFailed')}{summary.error ? ` · ${errorText({ code: summary.error })}` : ''}</span>
        {notConfigured
          ? <Button size="sm" variant="outline" className="h-6 text-xs" onClick={onOpenSettings}>{t('mail:summaryOpenSettings')}</Button>
          : <Button size="sm" variant="outline" className="h-6 text-xs" onClick={summary.resume}>{t('mail:summaryRetry')}</Button>}
      </div>
    )
  } else if (working && !summary.text) {
    body = <span className="text-muted-foreground">{t('mail:summaryWorking')}</span>
  } else {
    body = collapsed && !working
      ? <p className="truncate" title={summary.text}>{summary.text.replace(/^[-*]\s+/gm, '').replace(/\*\*/g, '').split('\n').filter(Boolean).join(' ').replace(/([。！？；：，])\s+/g, '$1')}</p>
      : <>
        <Markdown text={summary.text} streaming={working} onOpenLink={openLink} className="text-sm" />
        {summary.cut && <p className="mt-1 text-xs text-muted-foreground">{t('mail:summaryCut')}</p>}
      </>
  }

  return (
    <section aria-label={t('mail:summaryCardLabel')} aria-busy={working || undefined} className="relative isolate w-full shrink-0 animate-in rounded-lg text-sm duration-200 fade-in-0 slide-in-from-top-1">
      {/* The chat input's treatment: a faint glow behind while writing, and the AI gradient edge, drifting while writing. */}
      <div aria-hidden className={cn('ai-glow absolute -inset-0.5 -z-10 rounded-[inherit] opacity-0 transition-opacity', reduced ? 'duration-100' : 'duration-150', working && 'opacity-[0.28]')} />
      <div className="relative rounded-[inherit] bg-muted px-3 py-2">
      <div className="flex h-6 items-center gap-1.5 text-xs">
        {phase === 'done' ? <AiGeneratedMark at={summary.at} /> : <AiMark />}
        <span className="font-medium">{t('mail:summaryTitle')}</span>
        {styleName && <span className="text-muted-foreground">· {styleName}</span>}
        <span className="flex-1" />
        {working && <Button size="sm" variant="ghost" className="h-6 px-2 text-xs" onClick={summary.stop}>{t('mail:summaryStop')}</Button>}
        {phase === 'done' && <>
          <IconButton size="icon-sm" className={action} label={t('mail:summaryCopy')} onClick={copy}><HugeiconsIcon icon={Copy01Icon} /></IconButton>
          <IconButton size="icon-sm" className={action} label={t('mail:summaryRegenerate')} onClick={summary.regenerate}><HugeiconsIcon icon={RefreshIcon} /></IconButton>
          {onAsk && <Button size="sm" variant="ghost" className="h-6 px-2 text-xs text-muted-foreground" onClick={onAsk}>{t('mail:summaryAsk')}</Button>}
          <IconButton size="icon-sm" className={action} label={collapsed ? t('mail:summaryExpand') : t('mail:summaryCollapse')} aria-expanded={!collapsed} onClick={summary.toggleCollapsed}>
            <HugeiconsIcon icon={collapsed ? ArrowDown01Icon : ArrowUp01Icon} />
          </IconButton>
        </>}
        <IconButton size="icon-sm" className={action} label={t('mail:summaryClose')} onClick={summary.dismiss}><HugeiconsIcon icon={Cancel01Icon} /></IconButton>
      </div>
      <div className={cn('mt-1 overflow-y-auto leading-relaxed', !collapsed && 'max-h-60')}>{body}</div>
      <div aria-hidden data-slot="ai-border" data-flow={working && !reduced && visible} className="ai-border pointer-events-none absolute inset-0 rounded-[inherit]" />
      </div>
      <AiConsentDialog client={client} service={summary.service} open={consenting} onOpenChange={setConsenting} onGranted={summary.resume} />
    </section>
  )
}
