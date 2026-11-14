import { fireEvent, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { createMockClient } from '@/lib/mock-client'
import { withTitleBar } from '@/test/title-bar'
import { Workbench } from './workbench'

/** The workbench with its sidebar folded to the rail, as saved from an earlier session. */
async function renderRail() {
  const client = createMockClient()
  await client.updateSettings({ layout: { navCollapsed: true } })
  render(withTitleBar(<Workbench client={client} />))
  await screen.findByRole('list', { name: '邮件列表' })
  await screen.findByRole('button', { name: '展开侧边栏' })
  return within(screen.getByRole('navigation', { name: '邮箱导航' }))
}

/**
 * jsdom has no layout, so every pointer lands on (0, 0), which the list/reader
 * resize handle claims (it calls `preventDefault`, and the menu then stays
 * shut). A plain click carries no pointer position and opens the menu as a
 * real click on the icon would.
 */
const openMenu = (trigger: HTMLElement) => fireEvent.click(trigger)

describe('mail rail', () => {
  it('keeps the top views as icons, named with their unread number', async () => {
    const rail = await renderRail()
    const inbox = rail.getByRole('button', { name: /^收件箱 · \d+ 封未读$/ })
    expect(inbox).toHaveAttribute('aria-current', 'page')
    expect(rail.getByRole('button', { name: /^写邮件/ })).toBeInTheDocument()
    for (const name of ['星标', '已发送', '更多', '账号', '设置']) expect(rail.getByRole('button', { name })).toBeInTheDocument()
    expect(rail.queryByText('邮箱')).not.toBeInTheDocument()
    // 此刻 sits between compose and the inbox, with no number.
    const names = rail.getAllByRole('button').map((button) => button.getAttribute('aria-label') ?? '')
    expect(names.indexOf('此刻')).toBe(names.findIndex((name) => name.startsWith('写邮件')) + 1)
    await userEvent.click(rail.getByRole('button', { name: '此刻' }))
    expect(rail.getByRole('button', { name: '此刻' })).toHaveAttribute('aria-current', 'page')

    await userEvent.click(rail.getByRole('button', { name: '星标' }))
    expect(await screen.findByRole('heading', { name: '星标' })).toBeInTheDocument()
    expect(rail.getByRole('button', { name: '星标' })).toHaveAttribute('aria-current', 'page')
  })

  it('opens archive, junk and trash from the 更多 menu', async () => {
    const rail = await renderRail()
    openMenu(rail.getByRole('button', { name: '更多' }))
    await userEvent.click(await screen.findByRole('menuitem', { name: '已删除' }))
    expect(await screen.findByRole('heading', { name: '全部已删除' })).toBeInTheDocument()
    await waitFor(() => expect(screen.queryByRole('menu')).not.toBeInTheDocument())
    expect(rail.getByRole('button', { name: '更多' })).toHaveAttribute('data-active')
  })

  it('lists an account folders in a menu and marks the account of the open folder', async () => {
    const rail = await renderRail()
    openMenu(rail.getByRole('button', { name: /^工作邮箱/ }))
    const menu = await screen.findByRole('menu')
    for (const name of [/^收件箱/, /^已归档/, /^已发送/, /^MailX/, /^华东区/]) expect(within(menu).getByRole('menuitem', { name })).toBeInTheDocument()
    await userEvent.click(within(menu).getByRole('menuitem', { name: /^MailX/ }))
    expect(await screen.findByRole('heading', { name: 'MailX' })).toBeInTheDocument()
    expect(rail.getByRole('button', { name: /^工作邮箱/ })).toHaveAttribute('data-active')
  })
})
