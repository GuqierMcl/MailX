import { act, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createMockClient } from '@/lib/mock-client'
import type { MailxClient } from '@/lib/mailx-client'
import { withTitleBar } from '@/test/title-bar'
import { LAST_VISIT_KEY } from '@/features/now/visit'
import { Workbench } from './workbench'

const nav = () => within(screen.getByRole('navigation', { name: '邮箱导航' }))

async function startOnNow(client = createMockClient()) {
  const { settings } = await client.getSettings()
  await client.updateSettings({ general: { ...settings.general, startupView: 'now' } })
  return client
}

afterEach(() => localStorage.clear())

describe('Now page in the workbench', () => {
  it('opens from the sidebar between compose and the inbox, without the reader toggle', async () => {
    render(withTitleBar(<Workbench client={createMockClient()} />))
    await screen.findByRole('list', { name: '邮件列表' })
    expect(await screen.findByRole('button', { name: '收起邮件内容' })).toBeInTheDocument()
    const buttons = nav().getAllByRole('button').map((button) => button.textContent)
    expect(buttons.indexOf('此刻')).toBe(buttons.findIndex((label) => label?.startsWith('写邮件')) + 1)
    expect(buttons.indexOf('此刻')).toBeLessThan(buttons.indexOf('收件箱'))

    await userEvent.click(nav().getByRole('button', { name: '此刻' }))
    expect(await screen.findByRole('heading', { level: 1 })).toBeInTheDocument()
    expect(nav().getByRole('button', { name: '此刻' })).toHaveAttribute('data-active')
    expect(screen.queryByRole('list', { name: '邮件列表' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /邮件内容$/ })).not.toBeInTheDocument()
    // The title-bar search speaks for all inboxes, as in drafts and the outbox.
    expect(screen.getByRole('button', { name: '搜索邮件' })).toHaveTextContent('全部收件箱')

    await userEvent.click(nav().getByRole('button', { name: '收件箱' }))
    expect(await screen.findByRole('list', { name: '邮件列表' })).toBeInTheDocument()
    expect(await screen.findByRole('button', { name: '收起邮件内容' })).toBeInTheDocument()
  })

  it('ignores list and reader keys but keeps search and help', async () => {
    render(withTitleBar(<Workbench client={await startOnNow()} />))
    await screen.findByRole('heading', { level: 1 })
    await userEvent.keyboard('jo')
    expect(screen.getByRole('heading', { level: 1 })).toBeInTheDocument()
    expect(screen.queryByRole('toolbar', { name: '邮件操作' })).not.toBeInTheDocument()
    await userEvent.keyboard('/')
    expect(await screen.findByRole('combobox')).toBeInTheDocument()
  })

  it('starts on the Now page when the settings say so', async () => {
    render(withTitleBar(<Workbench client={await startOnNow()} />))
    expect(await screen.findByRole('heading', { level: 1 })).toBeInTheDocument()
    expect(nav().getByRole('button', { name: '此刻' })).toHaveAttribute('data-active')
  })

  it('does not take over a view the user picked before the settings arrived', async () => {
    const client = await startOnNow()
    const settings = await client.getSettings()
    let release!: () => void
    const held = new Promise<void>((resolve) => { release = resolve })
    const slow: MailxClient = { ...client, getSettings: vi.fn(async () => { await held; return settings }) }
    render(withTitleBar(<Workbench client={slow} />))
    await userEvent.click(await nav().findByRole('button', { name: '星标' }))
    await act(async () => release())
    await waitFor(() => expect(slow.getSettings).toHaveBeenCalled())
    expect(await screen.findByRole('heading', { name: '星标' })).toBeInTheDocument()
    expect(nav().getByRole('button', { name: '星标' })).toHaveAttribute('data-active')
    expect(nav().getByRole('button', { name: '此刻' })).not.toHaveAttribute('data-active')
  })

  it('opens a new message in the unified inbox and records the visit on leaving', async () => {
    const since = new Date(2000, 0, 1).toISOString()
    localStorage.setItem(LAST_VISIT_KEY, since)
    render(withTitleBar(<Workbench client={await startOnNow()} />))
    const section = await screen.findByRole('region', { name: '自上次查看后的新邮件' })
    const row = within(section).getAllByRole('button')[0]
    const subject = row.querySelector('span.truncate.text-muted-foreground')!.textContent!
    await userEvent.click(row)
    expect(await screen.findByRole('heading', { level: 2, name: subject })).toBeInTheDocument()
    expect(nav().getAllByRole('button', { name: /^收件箱/ })[0]).toHaveAttribute('data-active')
    expect(Date.parse(localStorage.getItem(LAST_VISIT_KEY)!)).toBeGreaterThan(Date.parse(since))
  })
})
