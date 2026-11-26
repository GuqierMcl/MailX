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
  it('groups pages under headings, with 关于 in the application group', () => {
    const { dialog } = renderMcp(testClient(), { section: 'general' })
    const nav = within(dialog).getByRole('navigation', { name: '设置' })
    const groups = within(nav).getAllByTestId(/^settings-nav-group-/)
    expect(groups.map((group) => group.getAttribute('data-testid'))).toEqual(['settings-nav-group-app', 'settings-nav-group-mail', 'settings-nav-group-integrations'])
    const names = (group: HTMLElement) => within(group).getAllByRole('button').map((button) => button.textContent)
    expect(groups.map(names)).toEqual([['通用', '外观', '此刻', '快捷键', '通知', '关于'], ['发信', '联系人', '增强', '安全性', '翻译'], ['AI', 'MCP']])
    expect(groups.map((group) => group.firstElementChild?.textContent)).toEqual(['应用', '邮件', '集成'])
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
  it('lists all requested agents and keeps manual setup explicit', async () => {
    const { client } = renderMcp()
    for (const name of ['Claude Code', 'Codex', 'Cursor', 'OpenCode', 'Trae', 'DeepSeek Harness']) {
      expect(await screen.findByText(name)).toBeInTheDocument()
    }
    expect(screen.queryByRole('button', { name: '接入 DeepSeek Harness' })).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: '复制 DeepSeek Harness 配置' })).toBeInTheDocument()
    expect(client.setMcpEnabled).not.toHaveBeenCalled()
  })

  it('adds a client configuration without claiming a live connection or enabling MCP', async () => {
    const { client } = renderMcp()
    await userEvent.click(await screen.findByRole('button', { name: '接入 Claude Code' }))
    expect(client.setupMcpAgent).toHaveBeenCalledWith('claudeCode', false)
    expect(await screen.findByText('已添加配置，请重载客户端以连接。')).toBeInTheDocument()
    expect(screen.getByRole('status', { name: '状态' })).toHaveTextContent('已关闭')
    expect(screen.queryByRole('list', { name: '已连接的客户端' })).not.toBeInTheDocument()
    expect(client.setMcpEnabled).not.toHaveBeenCalled()
  })

  it('prevents duplicate setup while a host operation is pending', async () => {
    const client = testClient()
    let finish: () => void = () => undefined
    const agent = (await client.getMcpAgents())[0]
    client.setupMcpAgent.mockImplementation(() => new Promise((resolve) => { finish = () => resolve({ state: 'configured', agent: { ...agent, configuration: 'configured' } }) }))
    renderMcp(client)
    const connect = await screen.findByRole('button', { name: '接入 Claude Code' })
    await userEvent.dblClick(connect)
    expect(client.setupMcpAgent).toHaveBeenCalledOnce()
    expect(screen.getByRole('button', { name: '接入 Codex' })).toBeDisabled()
    finish()
    expect(await screen.findByText('已添加配置，请重载客户端以连接。')).toBeInTheDocument()
  })

  it.each(['Cursor', 'Trae'])('leaves a %s import awaiting its client confirmation', async (name) => {
    renderMcp()
    await userEvent.click(await screen.findByRole('button', { name: `接入 ${name}` }))
    expect(await screen.findByText('已打开客户端导入，请在客户端中完成确认。')).toBeInTheDocument()
    expect(screen.queryByText('已添加配置')).not.toBeInTheDocument()
    expect(screen.queryByRole('list', { name: '已连接的客户端' })).not.toBeInTheDocument()
  })

  it('requires an explicit choice before replacing a conflicting client entry', async () => {
    const client = testClient()
    const agents = await client.getMcpAgents()
    client.getMcpAgents.mockResolvedValue(agents.map((agent) => agent.id === 'codex' ? { ...agent, configuration: 'conflict' } : agent))
    renderMcp(client)
    await userEvent.click(await screen.findByRole('button', { name: '接入 Codex' }))
    const first = await screen.findByRole('alertdialog')
    expect(first).toHaveTextContent('其他 MailX 安装位置或数据目录')
    expect(client.setupMcpAgent).not.toHaveBeenCalled()
    await userEvent.click(within(first).getByRole('button', { name: '取消' }))
    expect(client.setupMcpAgent).not.toHaveBeenCalled()
    await userEvent.click(screen.getByRole('button', { name: '接入 Codex' }))
    await userEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: '替换 MailX 条目' }))
    await waitFor(() => expect(client.setupMcpAgent).toHaveBeenCalledWith('codex', true))
  })

  it('removes only the selected client configuration', async () => {
    const { client } = renderMcp()
    await userEvent.click(await screen.findByRole('button', { name: '接入 Claude Code' }))
    await userEvent.click(await screen.findByRole('button', { name: '移除 Claude Code 配置' }))
    expect(client.removeMcpAgent).toHaveBeenCalledWith('claudeCode')
    expect(await screen.findByText('已移除 MailX 配置，请重载客户端以断开连接。')).toBeInTheDocument()
    expect(client.setMcpEnabled).not.toHaveBeenCalled()
  })

  it('checks the bridge without presenting the check as a live agent connection', async () => {
    const { client } = renderMcp()
    await userEvent.click(screen.getByRole('button', { name: '检查 MailX 连接' }))
    expect(await screen.findByText('MailX 连接正常；智能体尚未连接时，请重载客户端。')).toBeInTheDocument()
    expect(client.diagnoseMcp).toHaveBeenCalledOnce()
    expect(client.getEmail).not.toHaveBeenCalled()
    expect(client.listEmails).not.toHaveBeenCalled()
    expect(screen.queryByRole('list', { name: '已连接的客户端' })).not.toBeInTheDocument()
  })

  it('gives the next step when the server is disabled and when setup fails', async () => {
    const client = testClient()
    client.diagnoseMcp.mockResolvedValue({ ok: false, code: 'disabled' })
    client.setupMcpAgent.mockRejectedValue(new Error('failed'))
    renderMcp(client)
    await userEvent.click(screen.getByRole('button', { name: '检查 MailX 连接' }))
    expect(await screen.findByText('请先在上方开启 MCP 服务器，再重新检查。')).toBeInTheDocument()
    await userEvent.click(await screen.findByRole('button', { name: '接入 OpenCode' }))
    expect(await screen.findByText('无法完成 MCP 接入')).toBeInTheDocument()
    expect(screen.queryByText('已添加配置')).not.toBeInTheDocument()
  })

  it('keeps an import failure on its client row and clears it while retrying', async () => {
    const client = testClient()
    const agent = (await client.getMcpAgents()).find((agent) => agent.id === 'cursor')!
    client.setupMcpAgent.mockResolvedValueOnce({ state: 'unavailable', agent })
    renderMcp(client)
    const connect = await screen.findByRole('button', { name: '接入 Cursor' })
    const row = connect.closest('[data-slot="field"]') as HTMLElement
    await userEvent.click(connect)
    expect(await within(row).findByRole('alert')).toHaveTextContent('无法自动接入，请复制配置后手动添加。')
    const otherRow = screen.getByRole('button', { name: '接入 Codex' }).closest('[data-slot="field"]') as HTMLElement
    expect(within(otherRow).queryByRole('alert')).not.toBeInTheDocument()

    let finish: () => void = () => undefined
    client.setupMcpAgent.mockImplementationOnce(() => new Promise((resolve) => { finish = () => resolve({ state: 'handoff', agent }) }))
    await userEvent.click(connect)
    expect(within(row).queryByRole('alert')).not.toBeInTheDocument()
    finish()
    expect(await within(row).findByText('已打开客户端导入，请在客户端中完成确认。')).toBeInTheDocument()
    expect(within(row).queryByText('未配置')).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '检查配置' }))
    await waitFor(() => expect(within(row).queryByText('已打开客户端导入，请在客户端中完成确认。')).not.toBeInTheDocument())
  })

  it('keeps thrown diagnostics errors in the connection check group', async () => {
    const client = testClient()
    client.diagnoseMcp.mockRejectedValueOnce(new Error('diagnostic failed'))
    renderMcp(client)
    const check = screen.getByRole('button', { name: '检查 MailX 连接' })
    const group = check.closest('section') as HTMLElement
    await userEvent.click(check)
    expect(await within(group).findByRole('alert')).toBeInTheDocument()
    const agentGroup = (await screen.findByRole('button', { name: '接入 Cursor' })).closest('section') as HTMLElement
    expect(within(agentGroup).queryByRole('alert')).not.toBeInTheDocument()
  })

  it('copies the selected client format instead of the generic JSON', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true })
    renderMcp()
    await userEvent.click(await screen.findByRole('button', { name: '复制 Codex 配置' }))
    expect(writeText).toHaveBeenLastCalledWith(expect.stringContaining('[mcp_servers.mailx]'))
    await userEvent.click(screen.getByRole('button', { name: '复制 OpenCode 配置' }))
    expect(JSON.parse(writeText.mock.calls.at(-1)![0] as string)).toMatchObject({ mcp: { servers: { mailx: { type: 'local', command: ['/opt/MailX/mailx-mcp', '--discovery', '/data/mcp-discovery.json'] } } } })
  })

  it('asks for confirmation before turning the server on, and cancel keeps it off', async () => {
    const { client } = renderMcp()
    const toggle = await screen.findByRole('switch', { name: '开启 MCP 服务器' })
    await userEvent.click(toggle)
    const confirm = await screen.findByRole('alertdialog')
    expect(confirm).toHaveTextContent('读取所有账号的邮件和草稿')
    expect(confirm).toHaveTextContent('创建、修改和删除草稿')
    expect(confirm).toHaveTextContent('直接发送邮件，无需逐封确认')
    expect(confirm).toHaveTextContent('已读和星标会尝试同步到服务器')
    expect(confirm).toHaveTextContent('归档仅在本机生效')
    expect(confirm).toHaveTextContent('不能删除邮件')
    expect(confirm).toHaveTextContent('同一系统用户下的程序都可连接')
    expect(confirm).toHaveTextContent('读取本机文件作为附件')
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
})
