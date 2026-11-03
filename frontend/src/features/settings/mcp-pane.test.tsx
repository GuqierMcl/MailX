import { render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { testClient } from '@/features/accounts/test-fixtures'
import type { McpStatus } from '@/features/mcp/types'
import { Toaster } from '@/components/ui/toast'
import { SettingsDialog, type SettingsTarget } from './settings-dialog'

const SNIPPET = JSON.stringify({ mcpServers: { mailx: { command: '/opt/MailX/mailx-mcp', args: ['--discovery', '/data/mcp-discovery.json'] } } }, null, 2)

function renderMcp(client = testClient(), target: SettingsTarget = { section: 'mcp' }) {
  render(<><SettingsDialog client={client} target={target} onClose={vi.fn()} /><Toaster /></>)
  return { client, dialog: screen.getByRole('dialog', { name: '设置' }) }
}

/** A client whose status the test pushes by hand. */
function pushableClient(initial: McpStatus = { state: 'off' }) {
  const client = testClient()
  let listener: (status: McpStatus) => void = () => undefined
  client.getMcpStatus.mockResolvedValue(initial)
  client.onMcpStatus.mockImplementation((next) => { listener = next; return () => undefined })
  return { client, push: (status: McpStatus) => listener(status) }
}

afterEach(() => vi.restoreAllMocks())

describe('settings dialog navigation', () => {
  it('groups pages under headings, with 关于 alone at the bottom', () => {
    const { dialog } = renderMcp(testClient(), { section: 'general' })
    const groups = within(dialog).getAllByTestId(/^settings-nav-group-/)
    expect(groups.map((group) => group.getAttribute('data-testid'))).toEqual(['settings-nav-group-app', 'settings-nav-group-mail', 'settings-nav-group-integrations'])
    const names = (group: HTMLElement) => within(group).getAllByRole('button').map((button) => button.textContent)
    expect(groups.map(names)).toEqual([['通用', '外观', '通知'], ['发信', '安全性'], ['MCP']])
    expect(groups.map((group) => group.firstElementChild?.textContent)).toEqual(['应用', '邮件', '集成'])
    const about = within(dialog).getByRole('button', { name: '关于' })
    expect(groups.some((group) => group.contains(about))).toBe(false)
    const all = within(dialog).getAllByRole('button').filter((button) => ['通用', 'MCP', '关于'].includes(button.textContent ?? ''))
    expect(all.at(-1)).toBe(about)
  })

  it('opens the MCP page for the mcp target and switches to it from the nav', async () => {
    const { dialog } = renderMcp(testClient(), { section: 'mcp' })
    expect(await within(dialog).findByRole('switch', { name: '开启 MCP 服务器' })).toBeInTheDocument()
    await userEvent.click(within(dialog).getByRole('button', { name: '通用' }))
    expect(within(dialog).queryByRole('switch', { name: '开启 MCP 服务器' })).not.toBeInTheDocument()
    await userEvent.click(within(dialog).getByRole('button', { name: 'MCP' }))
    expect(await within(dialog).findByRole('switch', { name: '开启 MCP 服务器' })).toBeInTheDocument()
  })
})

describe('MCP settings page', () => {
  it('asks for confirmation before turning the server on, and cancel keeps it off', async () => {
    const { client } = renderMcp()
    const toggle = await screen.findByRole('switch', { name: '开启 MCP 服务器' })
    await userEvent.click(toggle)
    const confirm = await screen.findByRole('alertdialog')
    expect(confirm).toHaveTextContent('读取所有账号的本地邮件和草稿')
    expect(confirm).toHaveTextContent('创建、修改和删除草稿')
    expect(confirm).toHaveTextContent('不能发送邮件，也不能删除或标记邮件')
    expect(confirm).toHaveTextContent('以同一系统用户运行的任何程序都可以连接')
    expect(confirm).toHaveTextContent('随时在这里关闭')
    expect(client.setMcpEnabled).not.toHaveBeenCalled()
    await userEvent.click(within(confirm).getByRole('button', { name: '取消' }))
    await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument())
    expect(client.setMcpEnabled).not.toHaveBeenCalled()
    expect(toggle).not.toBeChecked()
  })

  it('turns the server on after confirming and off again without asking', async () => {
    const { client } = renderMcp()
    const toggle = await screen.findByRole('switch', { name: '开启 MCP 服务器' })
    await userEvent.click(toggle)
    await userEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: '开启' }))
    await waitFor(() => expect(client.setMcpEnabled).toHaveBeenCalledWith(true))
    await waitFor(() => expect(toggle).toBeChecked())
    await userEvent.click(toggle)
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    await waitFor(() => expect(client.setMcpEnabled).toHaveBeenLastCalledWith(false))
    await waitFor(() => expect(toggle).not.toBeChecked())
  })

  it('shows a failed change and leaves the switch where it was', async () => {
    const client = testClient()
    client.setMcpEnabled.mockRejectedValue(new Error('boom'))
    renderMcp(client)
    const toggle = await screen.findByRole('switch', { name: '开启 MCP 服务器' })
    await userEvent.click(toggle)
    await userEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: '开启' }))
    expect(await screen.findByText('无法更改 MCP 服务器设置')).toBeInTheDocument()
    expect(toggle).not.toBeChecked()
  })

  it('renders off, running with the client name, and every failure reason', async () => {
    const { client, push } = pushableClient()
    renderMcp(client)
    expect(await screen.findByRole('status')).toHaveTextContent('已关闭')
    push({ state: 'running', clients: [] })
    expect(await screen.findByText('运行中，0 个客户端已连接')).toBeInTheDocument()
    expect(screen.queryByRole('list', { name: '已连接的客户端' })).not.toBeInTheDocument()
    push({ state: 'running', clients: [{ label: 'Claude Code' }, { label: null }, { label: 'Claude Code' }, { label: 'Cursor' }] })
    expect(await screen.findByText('运行中，4 个客户端已连接')).toBeInTheDocument()
    const list = screen.getByRole('list', { name: '已连接的客户端' })
    expect(within(list).getAllByRole('listitem').map((item) => item.textContent)).toEqual(['Claude Code ×2', '未知客户端', 'Cursor'])
    expect(screen.queryByText(/最近连接/)).not.toBeInTheDocument()
    const reasons = { endpointInUse: '已被占用', endpointCreateFailed: '无法创建本地通信端点', permissionDenied: '系统拒绝访问', discoveryWriteFailed: '无法写入连接信息文件', tokenUnavailable: '无法生成安全令牌', acceptFailed: '监听在运行中中断' }
    for (const [code, text] of Object.entries(reasons)) {
      push({ state: 'failed', code } as McpStatus)
      expect(await screen.findByText(new RegExp(`启动失败：.*${text}`))).toBeInTheDocument()
    }
  })

  it('copies the exact mcpServers snippet with --discovery and shows the command path', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true })
    renderMcp()
    await userEvent.click(screen.getByRole('button', { name: '复制客户端配置' }))
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(SNIPPET))
    expect(JSON.parse(writeText.mock.calls[0][0] as string)).toEqual({ mcpServers: { mailx: { command: '/opt/MailX/mailx-mcp', args: ['--discovery', '/data/mcp-discovery.json'] } } })
    expect(await screen.findByText('已复制客户端配置')).toBeInTheDocument()
  })

  it('keeps manual setup collapsed, shows shortened paths and copies full ones', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true })
    renderMcp()
    const trigger = await screen.findByRole('button', { name: '手动配置' })
    expect(trigger).toHaveAttribute('aria-expanded', 'false')
    expect(screen.queryByTestId('mcp-command-path')).not.toBeInTheDocument()
    await userEvent.click(trigger)
    expect(await screen.findByTestId('mcp-command-path')).toHaveTextContent('/opt/MailX/mailx-mcp')
    expect(screen.getByTestId('mcp-discovery-path')).toHaveTextContent('~/data/mcp-discovery.json')
    await userEvent.click(screen.getByRole('button', { name: '复制连接信息文件路径' }))
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith('/data/mcp-discovery.json'))
    await userEvent.click(screen.getByRole('button', { name: '复制程序路径' }))
    await waitFor(() => expect(writeText).toHaveBeenLastCalledWith('/opt/MailX/mailx-mcp'))
  })

  it('leads to the General page from the lifecycle note', async () => {
    const { dialog } = renderMcp()
    await userEvent.click(await within(dialog).findByRole('button', { name: '前往「通用」设置' }))
    expect(await within(dialog).findByText('语言')).toBeInTheDocument()
  })
})
