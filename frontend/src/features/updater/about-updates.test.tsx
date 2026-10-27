import { render, screen, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { testClient } from '@/features/accounts/test-fixtures'
import { SettingsDialog } from '@/features/settings/settings-dialog'
import { UpdaterProvider } from './updater-provider'
import type { UpdateSnapshot } from './types'

function renderAbout(update: UpdateSnapshot) {
  const client = testClient()
  client.getUpdateState.mockResolvedValue(update)
  render(<UpdaterProvider client={client}><SettingsDialog client={client} target={{ section: 'about' }} onClose={() => undefined} /></UpdaterProvider>)
  return { client, dialog: screen.getByRole('dialog', { name: '设置' }) }
}

describe('the About page', () => {
  it('has a check button and the automatic check switch, but no last-check time', async () => {
    const { client, dialog } = renderAbout({ canInstall: true, phase: 'idle' })
    expect(within(dialog).getByText('0.1.0-alpha.2')).toBeInTheDocument()
    expect(within(dialog).getByRole('button', { name: '检查更新' })).toBeInTheDocument()
    expect(await within(dialog).findByRole('switch', { name: '自动检查更新' })).toBeChecked()
    // A last-check time is deliberately not shown anywhere.
    expect(dialog.textContent).not.toMatch(/上次检查|Last checked/)
    await userEvent.click(within(dialog).getByRole('switch', { name: '自动检查更新' }))
    expect(client.updateSettings).toHaveBeenCalledWith(expect.objectContaining({ general: expect.objectContaining({ autoCheckUpdates: false }) }))
  })

  it('says so when a manual check finds nothing', async () => {
    const { client, dialog } = renderAbout({ canInstall: true, phase: 'idle' })
    client.checkForUpdates.mockResolvedValue({ canInstall: true, phase: 'idle' })
    await userEvent.click(within(dialog).getByRole('button', { name: '检查更新' }))
    expect(await within(dialog).findByText('已是最新版本')).toBeInTheDocument()
  })

  it('links to the update when one is found', async () => {
    const { dialog } = renderAbout({ canInstall: true, phase: 'available', version: '0.2.0', current: '0.1.0', notes: null, date: null })
    await userEvent.click(await within(dialog).findByRole('button', { name: '有可用更新：0.2.0' }))
    expect(await screen.findByRole('dialog', { name: 'MailX 0.2.0' })).toBeInTheDocument()
  })

  it('shows a failed check with a retry', async () => {
    const { client, dialog } = renderAbout({ canInstall: true, phase: 'failed', stage: 'check', version: null })
    expect(await within(dialog).findByText('无法检查更新，请稍后重试。')).toBeInTheDocument()
    await userEvent.click(within(dialog).getByRole('button', { name: '重试' }))
    expect(client.checkForUpdates).toHaveBeenCalledOnce()
  })
})
