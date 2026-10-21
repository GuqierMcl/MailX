import { render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { createMockClient } from '@/lib/mock-client'
import { withTitleBar } from '@/test/title-bar'
import { Workbench } from './workbench'

async function openPalette() {
  const client = createMockClient()
  render(withTitleBar(<Workbench client={client} />))
  await screen.findByRole('list', { name: '邮件列表' })
  await userEvent.click(screen.getByRole('button', { name: '搜索邮件' }))
  const input = await screen.findByRole('combobox')
  return { client, input, palette: screen.getByRole('dialog') }
}

describe('command palette', () => {
  it('opens from the title bar with jump targets and commands, and closes on Escape', async () => {
    const { palette } = await openPalette()
    expect(within(palette).getByText('跳转')).toBeInTheDocument()
    expect(within(palette).getByText('命令')).toBeInTheDocument()
    expect(within(palette).getByRole('option', { name: '全部收件箱' })).toBeInTheDocument()
    expect(within(palette).getByRole('option', { name: '打开设置' })).toBeInTheDocument()
    await userEvent.keyboard('{Escape}')
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
  })

  it('opens with Cmd+K and starts empty every time', async () => {
    const { input } = await openPalette()
    await userEvent.type(input, '路线图')
    await userEvent.keyboard('{Escape}')
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
    await userEvent.keyboard('{Meta>}k{/Meta}')
    expect(await screen.findByRole('combobox')).toHaveValue('')
  })

  it('previews matching messages and applies the search to the list on Enter', async () => {
    const { input } = await openPalette()
    await userEvent.type(input, '路线图')
    const result = await screen.findByRole('option', { name: /第三季度产品路线图评审/ })
    expect(result).toBeInTheDocument()
    // The list stays untouched while the palette previews.
    expect(within(screen.getByRole('list', { name: '邮件列表', hidden: true })).getAllByRole('listitem', { hidden: true }).length).toBeGreaterThan(1)
    await userEvent.keyboard('{Enter}')
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
    await waitFor(() => expect(within(screen.getByRole('list', { name: '邮件列表' })).getAllByRole('listitem')).toHaveLength(1))
    expect(screen.getByRole('button', { name: '搜索邮件' })).toHaveTextContent('路线图')
  })

  it('opens a previewed message with the search applied', async () => {
    const { input } = await openPalette()
    await userEvent.type(input, '路线图')
    await userEvent.click(await screen.findByRole('option', { name: /第三季度产品路线图评审/ }))
    expect(await screen.findByRole('heading', { level: 2, name: '第三季度产品路线图评审' })).toBeInTheDocument()
  })

  it('jumps to a mailbox', async () => {
    const { input } = await openPalette()
    await userEvent.type(input, '已发送')
    await userEvent.click(await screen.findByRole('option', { name: /工作邮箱 · 已发送/ }))
    expect(await screen.findByRole('heading', { name: '已发送' })).toBeInTheDocument()
  })

  it('runs commands: switching appearance and clearing an applied search', async () => {
    const { client, input } = await openPalette()
    await userEvent.type(input, '深色')
    await userEvent.click(await screen.findByRole('option', { name: '外观：深色' }))
    await waitFor(async () => expect((await client.getSettings()).settings.appearance.theme).toBe('dark'))

    await userEvent.click(screen.getByRole('button', { name: '搜索邮件' }))
    await userEvent.type(await screen.findByRole('combobox'), '路线图{Enter}')
    await waitFor(() => expect(screen.getByRole('button', { name: '搜索邮件' })).toHaveTextContent('路线图'))
    await userEvent.click(screen.getByRole('button', { name: '搜索邮件' }))
    await userEvent.click(await screen.findByRole('option', { name: /清除搜索/ }))
    await waitFor(() => expect(screen.getByRole('button', { name: '搜索邮件' })).toHaveTextContent('在 全部收件箱 中搜索'))
  })

  it('says so when nothing matches', async () => {
    const { input } = await openPalette()
    await userEvent.type(input, 'zzzzqqqq')
    expect(await screen.findByText('没有匹配的邮件、邮箱或命令')).toBeInTheDocument()
  })
})
