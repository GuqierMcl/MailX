import { render, screen, waitFor } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { testClient } from '@/features/accounts/test-fixtures'
import { MailEnhancementsPane } from './mail-enhancements-pane'

describe('mail enhancement settings', () => {
  it('defaults verification cleanup off and saves it independently of notifications', async () => {
    const client = testClient()
    const initial = await client.getSettings()
    await client.updateSettings({ notifications: { ...initial.settings.notifications, enabled: false } })
    client.updateSettings.mockClear()
    render(<MailEnhancementsPane client={client} />)
    const toggle = await screen.findByRole('switch', { name: '复制验证码后移到已删除' })
    await waitFor(() => expect(toggle).not.toBeDisabled())
    expect(toggle).not.toBeChecked()
    expect(screen.getByText(/邮件列表、阅读区或系统通知/)).toBeInTheDocument()
    await userEvent.click(toggle)
    await waitFor(() => expect(toggle).toBeChecked())
    expect(client.updateSettings).toHaveBeenLastCalledWith({ mail: { trashVerificationMailAfterCopy: true } })
    expect((await client.getSettings()).settings.notifications.enabled).toBe(false)
  })

  it('keeps verification cleanup off when saving fails', async () => {
    const client = testClient()
    client.updateSettings.mockRejectedValueOnce(new Error('save failed'))
    render(<MailEnhancementsPane client={client} />)
    const toggle = await screen.findByRole('switch', { name: '复制验证码后移到已删除' })
    await waitFor(() => expect(toggle).not.toBeDisabled())
    await userEvent.click(toggle)
    expect(await screen.findByText('设置保存失败')).toBeInTheDocument()
    expect(toggle).not.toBeChecked()
    expect((await client.getSettings()).settings.mail.trashVerificationMailAfterCopy).toBe(false)
  })

})
