import { render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { exampleAccount, testClient } from '@/features/accounts/test-fixtures'
import { DEFAULT_NOW } from '@/features/now/modules/defaults'
import { NOW_AMBIENT_KEY, NOW_AMBIENT_SCENE_KEY } from '@/features/now/ambient-preference'
import type { NowSettings } from '@/generated/accounts'
import { SettingsDialog } from './settings-dialog'

const lastNow = (client: ReturnType<typeof testClient>) => client.updateSettings.mock.lastCall?.[0].now

async function renderPane(now?: Partial<NowSettings>) {
  const client = testClient([exampleAccount()])
  client.listFolders.mockResolvedValue([])
  if (now) await client.updateSettings({ now: { ...DEFAULT_NOW, ...now } })
  client.updateSettings.mockClear()
  render(<SettingsDialog client={client} target={{ section: 'now' }} onClose={() => undefined} />)
  const dialog = screen.getByRole('dialog', { name: '设置' })
  const group = await within(dialog).findByRole('group', { name: '模块' })
  return { client, group }
}

const ID = { 验证码: 'codes', 草稿: 'drafts', 你不在的时候: 'away', 收件箱: 'inbox', 一键清理: 'cleanup', 近况: 'activity', 最近的附件: 'attachments' } as Record<string, string>
const names = (group: HTMLElement) => [...group.querySelectorAll('li label')].map((label) => ID[label.textContent!])

describe('Now settings', () => {
  it('is its own page in the app group, after Appearance, and not part of Appearance', async () => {
    const client = testClient([exampleAccount()])
    client.listFolders.mockResolvedValue([])
    render(<SettingsDialog client={client} target={{ section: 'appearance' }} onClose={() => undefined} />)
    const dialog = screen.getByRole('dialog', { name: '设置' })
    const appGroup = within(dialog).getByTestId('settings-nav-group-app')
    const labels = within(appGroup).getAllByRole('button').map((button) => button.textContent)
    expect(labels.indexOf('此刻')).toBe(labels.indexOf('外观') + 1)
    await within(dialog).findByRole('switch', { name: '深色' }).catch(() => undefined)
    expect(within(dialog).queryByRole('group', { name: '模块' })).not.toBeInTheDocument()
    await userEvent.click(within(appGroup).getByRole('button', { name: '此刻' }))
    expect(await within(dialog).findByRole('group', { name: '模块' })).toBeInTheDocument()
    expect(within(dialog).getByRole('heading', { name: '此刻' })).toBeInTheDocument()
  })

  it('lists the modules that can be hidden, all shown, and none that cannot', async () => {
    const { group } = await renderPane()
    expect(within(group).getByRole('radio', { name: '自动' })).toBeChecked()
    expect(names(group)).toEqual(['codes', 'drafts', 'away', 'inbox', 'cleanup', 'activity', 'attachments'])
    for (const name of ['验证码', '草稿', '你不在的时候', '收件箱', '一键清理', '近况', '最近的附件']) expect(within(group).getByRole('switch', { name: name })).toBeChecked()
    expect(within(group).queryByRole('switch', { name: '需要处理' })).not.toBeInTheDocument()
    // Nothing to reorder while the arrangement is automatic.
    expect(within(group).queryByRole('button', { name: /^上移/ })).not.toBeInTheDocument()
    expect(within(group).getByRole('button', { name: '恢复默认（此刻）' })).toBeDisabled()
  })

  it('saves a module turned off, keeping the rest of the group', async () => {
    const { client, group } = await renderPane({ hidden: ['activity'] })
    await userEvent.click(within(group).getByRole('switch', { name: '草稿' }))
    expect(lastNow(client)).toEqual({ arrangement: 'auto', order: [], hidden: ['activity', 'drafts'] })
    await userEvent.click(within(group).getByRole('switch', { name: '近况' }))
    expect(lastNow(client)).toEqual({ arrangement: 'auto', order: [], hidden: ['drafts'] })
  })

  it('switches to a fixed order, lists the modules in it and lets them move with the buttons', async () => {
    const { client, group } = await renderPane()
    await userEvent.click(within(group).getByRole('radio', { name: '固定顺序' }))
    expect(lastNow(client)).toEqual({ arrangement: 'fixed', order: [], hidden: [] })
    await waitFor(() => expect(within(group).getAllByRole('button', { name: /^调整 .* 的顺序$/ })).toHaveLength(7))
    expect(within(group).getByRole('button', { name: '上移 验证码' })).toBeDisabled()
    expect(within(group).getByRole('button', { name: '下移 最近的附件' })).toBeDisabled()

    await userEvent.click(within(group).getByRole('button', { name: '下移 验证码' }))
    expect(lastNow(client)).toEqual({ arrangement: 'fixed', order: ['drafts', 'codes', 'away', 'inbox', 'cleanup', 'activity', 'attachments'], hidden: [] })
    await waitFor(() => expect(names(group)).toEqual(['drafts', 'codes', 'away', 'inbox', 'cleanup', 'activity', 'attachments']))
    await userEvent.click(within(group).getByRole('button', { name: '上移 近况' }))
    expect(lastNow(client)?.order).toEqual(['drafts', 'codes', 'away', 'inbox', 'activity', 'cleanup', 'attachments'])
  })

  it('follows a saved order, keeps ids this version does not know, and keeps hidden ones listed', async () => {
    const { client, group } = await renderPane({ arrangement: 'fixed', order: ['activity', 'future', 'inbox'], hidden: ['inbox'] })
    expect(names(group)).toEqual(['activity', 'inbox', 'codes', 'drafts', 'away', 'cleanup', 'attachments'])
    expect(within(group).getByRole('switch', { name: '收件箱' })).not.toBeChecked()
    await userEvent.click(within(group).getByRole('button', { name: '上移 收件箱' }))
    expect(lastNow(client)?.order).toEqual(['inbox', 'activity', 'codes', 'drafts', 'away', 'cleanup', 'attachments', 'future'])
  })

  it('restores the defaults for this group only', async () => {
    const { client, group } = await renderPane({ arrangement: 'fixed', order: ['activity'], hidden: ['codes'] })
    await userEvent.click(within(group).getByRole('button', { name: '恢复默认（此刻）' }))
    expect(lastNow(client)).toEqual(DEFAULT_NOW)
    await waitFor(() => expect(within(group).getByRole('radio', { name: '自动' })).toBeChecked())
    expect(within(group).getByRole('switch', { name: '验证码' })).toBeChecked()
  })

  it('shows a failed save beside the group', async () => {
    const { client, group } = await renderPane()
    client.updateSettings.mockRejectedValueOnce(new Error('disk full'))
    await userEvent.click(within(group).getByRole('switch', { name: '近况' }))
    expect(await within(group).findByText('未能保存设置')).toBeInTheDocument()
    expect(within(group).getByRole('switch', { name: '近况' })).toBeChecked()
  })

  it('lets the user switch the ambient backdrop mode and preview scene card', async () => {
    localStorage.clear()
    await renderPane()
    const dialog = screen.getByRole('dialog', { name: '设置' })
    const backdrop = within(dialog).getByRole('group', { name: '背景' })
    expect(within(backdrop).getByRole('radio', { name: '开启' })).toBeChecked()
    const sceneGroup = within(backdrop).getByRole('radiogroup', { name: '场景风格' })
    expect(within(sceneGroup).getByRole('radio', { name: /自动切换/ })).toBeChecked()
    await userEvent.click(within(sceneGroup).getByRole('radio', { name: '海面天光' }))
    expect(localStorage.getItem(NOW_AMBIENT_SCENE_KEY)).toBe('sea')
    await userEvent.click(within(backdrop).getByRole('radio', { name: '仅静态' }))
    expect(localStorage.getItem(NOW_AMBIENT_KEY)).toBe('static')
    await userEvent.click(within(backdrop).getByRole('radio', { name: '关闭' }))
    expect(localStorage.getItem(NOW_AMBIENT_KEY)).toBe('off')
    expect(within(backdrop).queryByRole('radiogroup', { name: '场景风格' })).not.toBeInTheDocument()
    localStorage.clear()
  })
})
