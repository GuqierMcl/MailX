import { StrictMode } from 'react'
import { useTranslation } from 'react-i18next'
import { describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { act, render, screen, waitFor } from '@/test/render'
import { testClient } from '@/features/accounts/test-fixtures'
import { GeneralPane } from '@/features/settings/general-pane'
import type { SettingsSnapshot } from '@/generated/accounts'
import { SettingsSync } from './settings-sync'
import { i18n } from './index'

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((done) => { resolve = done })
  return { promise, resolve }
}

function LanguageProbe() {
  const { t } = useTranslation('settings')
  return <div>{t('general')}</div>
}

describe('language settings synchronization', () => {
  it('waits for listener registration before reading, including StrictMode remounts', async () => {
    const client = testClient()
    const ready = deferred<() => void>()
    client.onSettingsChanged.mockReturnValue(ready.promise)
    render(<StrictMode><SettingsSync client={client} /></StrictMode>)
    expect(client.getSettings).not.toHaveBeenCalled()
    await act(async () => ready.resolve(() => undefined))
    await waitFor(() => expect(client.getSettings).toHaveBeenCalledOnce())
    expect(client.onSettingsChanged).toHaveBeenCalledOnce()
  })

  it('keeps a newer event snapshot when the initial request returns late', async () => {
    const client = testClient()
    const initial = await client.getSettings()
    client.getSettings.mockClear()
    const slow = deferred<SettingsSnapshot>()
    const latest: SettingsSnapshot = { ...initial, revision: 2, effectiveLocale: 'en', settings: { ...initial.settings, general: { language: 'en', autoCheckUpdates: true } } }
    client.getSettings.mockReturnValueOnce(slow.promise).mockResolvedValue(latest)
    let changed: ((change: { revision: number }) => void) | undefined
    client.onSettingsChanged.mockImplementation(async (listener) => { changed = listener; return () => undefined })
    render(<><SettingsSync client={client} /><LanguageProbe /></>)
    await waitFor(() => expect(client.getSettings).toHaveBeenCalledOnce())
    act(() => changed!({ revision: 2 }))
    expect(await screen.findByText('General')).toBeInTheDocument()
    await act(async () => slow.resolve(initial))
    expect(screen.getByText('General')).toBeInTheDocument()
    expect(document.documentElement.lang).toBe('en')
  })

  it('recovers a missed event on focus and cleans up a retried listener', async () => {
    const client = testClient()
    const stop = vi.fn()
    client.onSettingsChanged.mockRejectedValueOnce(new Error('temporary')).mockResolvedValue(stop)
    const view = render(<><SettingsSync client={client} /><LanguageProbe /></>)
    await waitFor(() => expect(client.getSettings).toHaveBeenCalled())
    const latest = await client.updateSettings({ general: { language: 'en', autoCheckUpdates: true } })
    client.getSettings.mockResolvedValue(latest)
    act(() => window.dispatchEvent(new Event('focus')))
    expect(await screen.findByText('General')).toBeInTheDocument()
    view.unmount()
    await act(async () => { await Promise.resolve() })
    expect(stop).toHaveBeenCalledOnce()
  })

  it('saves a language choice and changes text, document language and first-paint cache', async () => {
    const client = testClient()
    render(<><SettingsSync client={client} /><GeneralPane client={client} /></>)
    await userEvent.click(await screen.findByRole('combobox', { name: '语言' }))
    await userEvent.click(await screen.findByRole('option', { name: 'English' }))
    await waitFor(() => expect(i18n.language).toBe('en'))
    expect(client.updateSettings).toHaveBeenCalledWith({ general: { language: 'en', autoCheckUpdates: true } })
    expect(screen.getByRole('combobox', { name: 'Language' })).toHaveTextContent('English')
    expect(JSON.parse(localStorage.getItem('mailx.language')!)).toEqual({ language: 'en', effectiveLocale: 'en' })
  })

  it('keeps the previous language if saving fails', async () => {
    const client = testClient()
    client.updateSettings.mockRejectedValue(new Error('disk full'))
    render(<><SettingsSync client={client} /><GeneralPane client={client} /></>)
    await userEvent.click(await screen.findByRole('combobox', { name: '语言' }))
    await userEvent.click(await screen.findByRole('option', { name: 'English' }))
    expect(await screen.findByText('设置保存失败')).toBeInTheDocument()
    expect(i18n.language).toBe('zh-CN')
    expect(screen.getByRole('combobox', { name: '语言' })).toHaveTextContent('简体中文')
  })
})
