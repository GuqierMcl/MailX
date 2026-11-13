import { useState, type FormEvent, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import { errorText } from '@/i18n/errors'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import type { ProxyKind, ServiceRoute } from '@/generated/accounts'
import { SettingsRow } from './settings-layout'

/**
 * A secret typed once: not saved, a password field and Save; saved, only "Saved" with Replace and Remove.
 * The value never outlives the save attempt in the page, and a failed save keeps nothing in the field.
 * A key is trimmed; a password is stored as typed.
 */
export function StoredSecretRow({ id, label, description, placeholder, configured, trim = false, disabled = false, onSave, onRemove }: {
  id: string; label: string; description?: string; placeholder: string; configured: boolean; trim?: boolean; disabled?: boolean
  onSave: (value: string) => Promise<unknown>; onRemove: () => Promise<unknown>
}) {
  useTranslation()
  const [replacing, setReplacing] = useState(false)
  const [value, setValue] = useState('')
  const [busy, setBusy] = useState(false)
  const [failure, setFailure] = useState<string | null>(null)
  const ready = (trim ? value.trim() : value) !== ''
  const save = (event: FormEvent) => {
    event.preventDefault()
    if (!ready || busy || disabled) return
    setBusy(true)
    setFailure(null)
    onSave(trim ? value.trim() : value).then(() => setReplacing(false), (error: unknown) => setFailure(errorText(error))).finally(() => { setValue(''); setBusy(false) })
  }
  const remove = () => {
    setBusy(true)
    setFailure(null)
    onRemove().catch((error: unknown) => setFailure(errorText(error))).finally(() => setBusy(false))
  }
  const showSaved = configured && !replacing
  return <SettingsRow label={label} htmlFor={showSaved ? undefined : id} description={description}>
    <div className="flex flex-col items-end gap-1">
      {showSaved ? (
        <div className="flex items-center gap-2">
          <span className="text-xs text-muted-foreground">{t('settings:translationKeySaved')}</span>
          <Button variant="outline" size="sm" disabled={busy || disabled} onClick={() => setReplacing(true)}>{t('settings:translationKeyReplace')}</Button>
          <Button variant="outline" size="sm" disabled={busy} onClick={remove}>{t('settings:translationKeyRemove')}</Button>
        </div>
      ) : (
        <form className="flex items-center gap-2" onSubmit={save}>
          <Input id={id} type="password" autoComplete="off" spellCheck={false} className="w-48" value={value} disabled={busy || disabled}
            placeholder={placeholder} aria-invalid={failure !== null || undefined} onChange={(event) => setValue(event.target.value)} />
          <Button type="submit" variant="outline" size="sm" disabled={busy || disabled || !ready}>{t('settings:translationKeySave')}</Button>
          {replacing && <Button type="button" variant="ghost" size="sm" disabled={busy} onClick={() => { setReplacing(false); setValue(''); setFailure(null) }}>{t('settings:translationKeyCancel')}</Button>}
        </form>
      )}
      {failure && <p role="alert" className="max-w-72 text-right text-xs text-destructive">{failure}</p>}
    </div>
  </SettingsRow>
}

type NetworkEdit = { mode?: 'direct' | 'proxy'; proxyKind?: ProxyKind; host?: string; port?: string; username?: string }

/**
 * The connection rows shared by the translation and AI settings: direct or through a proxy.
 * `idPrefix` keeps the field ids apart; `password` renders the password row for the page's own store.
 */
export function ProxyRows({ idPrefix, route, networkDescription, onRouteChange, password }: {
  idPrefix: string; route: ServiceRoute; networkDescription: string
  onRouteChange: (route: ServiceRoute) => void
  password: (hasUsername: boolean) => ReactNode
}) {
  useTranslation()
  const [mode, setMode] = useState<'direct' | 'proxy'>(route.kind)
  const [proxyKind, setProxyKind] = useState<ProxyKind>(route.kind === 'proxy' ? route.proxyKind : 'socks5')
  const [host, setHost] = useState(route.kind === 'proxy' ? route.host : '')
  const [port, setPort] = useState(route.kind === 'proxy' ? String(route.port) : '')
  const [username, setUsername] = useState(route.kind === 'proxy' ? route.username : '')
  const modes: Record<string, string> = { direct: t('settings:translationNetworkDirect'), proxy: t('settings:translationNetworkProxy') }
  const kinds: Record<string, string> = { socks5: 'SOCKS5', httpConnect: 'HTTP' }
  const portNumber = /^\d{1,5}$/.test(port.trim()) ? Number(port.trim()) : 0
  const complete = host.trim() !== '' && portNumber >= 1 && portNumber <= 65535
  // Nothing half-filled is ever saved: until the address and port are right, the saved route stays as it was.
  const commit = (next: NetworkEdit) => {
    const kind = next.mode ?? mode
    if (kind === 'direct') { onRouteChange({ kind: 'direct' }); return }
    const nextHost = (next.host ?? host).trim()
    const nextPort = /^\d{1,5}$/.test((next.port ?? port).trim()) ? Number((next.port ?? port).trim()) : 0
    if (nextHost && nextPort >= 1 && nextPort <= 65535) {
      onRouteChange({ kind: 'proxy', proxyKind: next.proxyKind ?? proxyKind, host: nextHost, port: nextPort, username: (next.username ?? username).trim() })
    }
  }
  return <>
    <SettingsRow label={t('settings:translationNetwork')} htmlFor={`${idPrefix}.network`} description={networkDescription}>
      <Select value={mode} items={modes} onValueChange={(value: string | null) => {
        if (value !== 'direct' && value !== 'proxy') return
        setMode(value)
        commit({ mode: value })
      }}>
        <SelectTrigger id={`${idPrefix}.network`} className="w-56"><SelectValue /></SelectTrigger>
        <SelectContent>{Object.entries(modes).map(([value, label]) => <SelectItem key={value} value={value}>{label}</SelectItem>)}</SelectContent>
      </Select>
    </SettingsRow>
    {mode === 'proxy' && <>
      <SettingsRow label={t('settings:translationProxyType')} htmlFor={`${idPrefix}.proxyKind`}>
        <Select value={proxyKind} items={kinds} onValueChange={(value: string | null) => {
          if (value !== 'socks5' && value !== 'httpConnect') return
          setProxyKind(value)
          commit({ proxyKind: value })
        }}>
          <SelectTrigger id={`${idPrefix}.proxyKind`} className="w-56"><SelectValue /></SelectTrigger>
          <SelectContent>{Object.entries(kinds).map(([value, label]) => <SelectItem key={value} value={value}>{label}</SelectItem>)}</SelectContent>
        </Select>
      </SettingsRow>
      <SettingsRow label={t('settings:translationProxyHost')} htmlFor={`${idPrefix}.proxyHost`}>
        <Input id={`${idPrefix}.proxyHost`} className="w-56" value={host} autoComplete="off" spellCheck={false}
          onChange={(event) => setHost(event.target.value)} onBlur={() => commit({})} />
      </SettingsRow>
      <SettingsRow label={t('settings:translationProxyPort')} htmlFor={`${idPrefix}.proxyPort`}>
        <div className="flex flex-col items-end gap-1">
          <Input id={`${idPrefix}.proxyPort`} className="w-24" inputMode="numeric" value={port} autoComplete="off"
            onChange={(event) => setPort(event.target.value)} onBlur={() => commit({})} />
          {!complete && <p role="alert" className="max-w-56 text-right text-xs text-destructive">{t('settings:translationProxyInvalid')}</p>}
        </div>
      </SettingsRow>
      <SettingsRow label={t('settings:translationProxyUser')} htmlFor={`${idPrefix}.proxyUser`} description={t('settings:translationProxyUserDescription')}>
        <Input id={`${idPrefix}.proxyUser`} className="w-56" value={username} autoComplete="off" spellCheck={false}
          onChange={(event) => setUsername(event.target.value)} onBlur={() => commit({})} />
      </SettingsRow>
      {password(username.trim() !== '')}
    </>}
  </>
}

/** The proxy password row: without a username it would not be used, so it cannot be set. */
export function ProxyPasswordFields({ id, hasUsername, configured, onSave, onRemove }: {
  id: string; hasUsername: boolean; configured: boolean
  onSave: (value: string) => Promise<unknown>; onRemove: () => Promise<unknown>
}) {
  useTranslation()
  return <StoredSecretRow id={id} label={t('settings:translationProxyPassword')}
    description={hasUsername ? undefined : t('settings:translationProxyPasswordNeedsUser')} placeholder={t('settings:translationProxyPasswordPlaceholder')}
    configured={configured} disabled={!hasUsername} onSave={onSave} onRemove={onRemove} />
}
