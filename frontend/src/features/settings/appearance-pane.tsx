import { useEffect, useState, type KeyboardEvent } from 'react'
import { HugeiconsIcon, type IconSvgElement } from '@hugeicons/react'
import { ComputerIcon, Moon02Icon, Sun03Icon, Tick02Icon } from '@hugeicons/core-free-icons'
import { Radio } from '@base-ui/react/radio'
import { RadioGroup } from '@base-ui/react/radio-group'
import { cn } from 'cn'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Field, FieldDescription, FieldError, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Delayed } from '@/components/ui/delayed'
import { Spinner } from '@/components/ui/spinner'
import type { AppearanceSettings, FontSize, MailTheme, PreviewLines, ThemeMode } from '@/generated/accounts'
import { ACCENT_PRESETS, DEFAULT_INTERFACE_FONTS, DEFAULT_MAIL_FONTS, fontFamilyCss, parseFontFamilies } from '@/lib/appearance'
import { clientFailure, type MailxClient } from '@/lib/mailx-client'
import { SettingsPage, SettingsRow, SettingsGroup } from './settings-layout'
import { useSettings, useUpdateAppearance } from './settings-queries'

const THEME_OPTIONS: { value: ThemeMode; label: string; icon: IconSvgElement }[] = [
  { value: 'system', label: '跟随系统', icon: ComputerIcon },
  { value: 'light', label: '浅色', icon: Sun03Icon },
  { value: 'dark', label: '深色', icon: Moon02Icon },
]

const MAIL_THEME_LABELS: Record<MailTheme, string> = { follow: '跟随颜色模式', light: '始终白底' }

const FONT_SIZE_LABELS: Record<FontSize, string> = { small: '较小', default: '标准', large: '较大' }

const PREVIEW_LINE_LABELS: Record<PreviewLines, string> = { none: '不显示', one: '1 行', two: '2 行', three: '3 行' }

const FONT_PREVIEW = '敏捷的棕色狐狸跳过了懒狗 · The quick brown fox jumps over the lazy dog 0123456789'

export function AppearancePane({ client }: { client: MailxClient }) {
  const settings = useSettings(client)
  const update = useUpdateAppearance(client)

  if (settings.isPending) {
    return <Delayed><div className="flex justify-center p-10"><Spinner className="size-5 text-muted-foreground" /></div></Delayed>
  }
  if (settings.isError) {
    return (
      <SettingsPage>
        <Alert variant="destructive">
          <AlertTitle>无法读取设置</AlertTitle>
          <AlertDescription>{clientFailure(settings.error).message}</AlertDescription>
        </Alert>
        <Button variant="outline" className="self-start" onClick={() => void settings.refetch()}>重试</Button>
      </SettingsPage>
    )
  }

  const appearance = settings.data.settings.appearance
  const change = (patch: Partial<AppearanceSettings>) => update.mutate({ ...appearance, ...patch })
  const failure = update.error ? clientFailure(update.error) : null
  const fontRejected = failure?.field === 'appearance.interfaceFont'
  const mailFontRejected = failure?.field === 'appearance.mailFont'

  return (
    <SettingsPage>
      {settings.data.recovered && (
        <Alert>
          <AlertTitle>设置无法读取</AlertTitle>
          <AlertDescription>已改用默认设置，原来的设置文件保留为 settings.json.bak。</AlertDescription>
        </Alert>
      )}
      {failure && !fontRejected && !mailFontRejected && (
        <Alert variant="destructive">
          <AlertTitle>未能保存设置</AlertTitle>
          <AlertDescription>{failure.message}</AlertDescription>
        </Alert>
      )}

      <SettingsGroup title="颜色">
        <SettingsRow label="颜色模式">
          <ThemeChoice value={appearance.theme} onChange={(theme) => change({ theme })} />
        </SettingsRow>
        <SettingsRow label="深色模式下的邮件" description="有些邮件在深色下显示不佳，可改为白底。" htmlFor="appearance.mailTheme">
          <Select value={appearance.mailTheme} items={MAIL_THEME_LABELS}
            onValueChange={(mailTheme: MailTheme | null) => { if (mailTheme) change({ mailTheme }) }}>
            <SelectTrigger id="appearance.mailTheme" className="w-36"><SelectValue /></SelectTrigger>
            <SelectContent>
              {Object.entries(MAIL_THEME_LABELS).map(([value, label]) => <SelectItem key={value} value={value}>{label}</SelectItem>)}
            </SelectContent>
          </Select>
        </SettingsRow>
        <SettingsRow label="强调色">
          <AccentChoice value={appearance.accentColor} onChange={(accentColor) => change({ accentColor })} />
        </SettingsRow>
      </SettingsGroup>

      <SettingsGroup title="文字">
        <FontField
          id="appearance.interfaceFont"
          label="界面字体"
          value={appearance.interfaceFont}
          defaults={DEFAULT_INTERFACE_FONTS}
          rejected={fontRejected}
          onCommit={(interfaceFont) => change({ interfaceFont })}
        />
        <FontField
          id="appearance.mailFont"
          label="邮件字体"
          value={appearance.mailFont}
          defaults={DEFAULT_MAIL_FONTS}
          rejected={mailFontRejected}
          resetLabel="恢复默认邮件字体"
          onCommit={(mailFont) => change({ mailFont })}
        />
        <SettingsRow label="字号" htmlFor="appearance.fontSize">
          <Select value={appearance.fontSize} items={FONT_SIZE_LABELS}
            onValueChange={(fontSize: FontSize | null) => { if (fontSize) change({ fontSize }) }}>
            <SelectTrigger id="appearance.fontSize" className="w-28"><SelectValue /></SelectTrigger>
            <SelectContent>
              {Object.entries(FONT_SIZE_LABELS).map(([value, label]) => <SelectItem key={value} value={value}>{label}</SelectItem>)}
            </SelectContent>
          </Select>
        </SettingsRow>
      </SettingsGroup>

      <SettingsGroup title="邮件列表">
        <SettingsRow label="邮件预览行数" htmlFor="appearance.previewLines">
          <Select value={appearance.previewLines} items={PREVIEW_LINE_LABELS}
            onValueChange={(previewLines: PreviewLines | null) => { if (previewLines) change({ previewLines }) }}>
            <SelectTrigger id="appearance.previewLines" className="w-28"><SelectValue /></SelectTrigger>
            <SelectContent>
              {Object.entries(PREVIEW_LINE_LABELS).map(([value, label]) => <SelectItem key={value} value={value}>{label}</SelectItem>)}
            </SelectContent>
          </Select>
        </SettingsRow>
      </SettingsGroup>
    </SettingsPage>
  )
}

function ThemeChoice({ value, onChange }: { value: ThemeMode; onChange: (theme: ThemeMode) => void }) {
  return (
    <RadioGroup aria-label="颜色模式" value={value} onValueChange={(theme) => onChange(theme as ThemeMode)}
      className="inline-flex rounded-lg bg-muted p-0.5">
      {THEME_OPTIONS.map((option) => (
        <Radio.Root key={option.value} value={option.value}
          className="inline-flex h-7 items-center gap-1.5 rounded-md px-2.5 text-xs font-medium text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-3 focus-visible:ring-ring/50 data-checked:bg-background data-checked:text-foreground data-checked:shadow-sm">
          <HugeiconsIcon icon={option.icon} strokeWidth={2} className="size-3.5" />
          {option.label}
        </Radio.Root>
      ))}
    </RadioGroup>
  )
}

function AccentChoice({ value: saved, onChange }: { value: string; onChange: (accentColor: string) => void }) {
  // The native picker reports every drag step; save once it settles.
  const [picking, setPicking] = useState<string | null>(null)
  useEffect(() => {
    if (picking === null) return
    const timer = setTimeout(() => {
      onChange(picking)
      setPicking(null)
    }, 250)
    return () => clearTimeout(timer)
  }, [picking, onChange])
  const value = picking ?? saved
  const custom = value !== '' && !ACCENT_PRESETS.some((preset) => preset.value === value)
  const swatch = 'relative flex size-6 items-center justify-center rounded-full outline-none ring-offset-2 ring-offset-background transition-shadow focus-visible:ring-3 focus-visible:ring-ring/50 data-checked:ring-2 data-checked:ring-foreground/70'
  const tick = <HugeiconsIcon icon={Tick02Icon} strokeWidth={3} className="size-3.5 text-white mix-blend-difference" />
  return (
    <div className="flex flex-wrap items-center gap-2.5">
      <RadioGroup aria-label="强调色" value={custom ? null : value} onValueChange={(next) => onChange(next as string)}
        className="flex flex-wrap items-center gap-2.5">
        {ACCENT_PRESETS.map((preset) => {
          const label = preset.value ? preset.label : `${preset.label}（默认）`
          return (
            <Radio.Root key={preset.value || 'default'} value={preset.value} aria-label={label} title={label}
              className={cn(swatch, !preset.value && 'bg-neutral-900 dark:bg-neutral-100')}
              style={preset.value ? { backgroundColor: preset.value } : undefined}>
              <Radio.Indicator>{tick}</Radio.Indicator>
            </Radio.Root>
          )
        })}
      </RadioGroup>
      <span className="h-5 w-px bg-border" aria-hidden />
      {/* The native picker sits invisibly over the swatch so the OS color panel opens on click. */}
      <label title="自定义颜色" data-checked={custom || undefined}
        className={cn(swatch, 'cursor-pointer has-focus-visible:ring-3 has-focus-visible:ring-ring/50',
          !custom && 'bg-[conic-gradient(#ef4444,#eab308,#22c55e,#06b6d4,#3b82f6,#a855f7,#ef4444)]')}
        style={custom ? { backgroundColor: value } : undefined}>
        <input type="color" aria-label="自定义强调色" className="absolute inset-0 size-full cursor-pointer opacity-0"
          value={custom ? value : '#2563eb'} onChange={(event) => setPicking(event.target.value.toLowerCase())} />
        {custom && <span className="pointer-events-none">{tick}</span>}
      </label>
    </div>
  )
}

function FontField({ id, label, value, defaults, rejected, resetLabel, onCommit }: {
  id: string
  label: string
  value: string
  defaults: string[]
  rejected: boolean
  resetLabel?: string
  onCommit: (font: string) => void
}) {
  // Only this field edits the font, so the draft is local; a rejected list
  // stays in the input for correction while the applied font rolls back.
  const [draft, setDraft] = useState(value)
  const commit = () => {
    const normalized = parseFontFamilies(draft).join(', ')
    setDraft(normalized)
    if (normalized !== value) onCommit(normalized)
  }
  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'Enter') commit()
    if (event.key === 'Escape' && draft !== value) {
      // Discard the edit instead of letting the dialog close.
      event.stopPropagation()
      setDraft(value)
    }
  }
  return (
    <Field data-invalid={rejected || undefined}>
      <FieldLabel htmlFor={id}>{label}</FieldLabel>
      <div className="flex gap-2">
        <Input id={id} value={draft} placeholder={`默认：${defaults.slice(0, 2).join(', ')} …`}
          spellCheck={false} autoComplete="off" aria-invalid={rejected || undefined}
          onChange={(event) => setDraft(event.target.value)} onBlur={commit} onKeyDown={onKeyDown} />
        <Button variant="outline" aria-label={resetLabel} disabled={!value && !draft} onClick={() => { setDraft(''); if (value) onCommit('') }}>恢复默认</Button>
      </div>
      <FieldDescription>可填多个，用英文逗号隔开。</FieldDescription>
      {rejected && <FieldError>字体名里有不能使用的字符，或写得太多、太长。</FieldError>}
      <p className="rounded-lg border bg-muted/40 px-3 py-2.5 text-sm text-foreground" style={{ fontFamily: fontFamilyCss(draft, defaults) }}>
        {FONT_PREVIEW}
      </p>
    </Field>
  )
}
