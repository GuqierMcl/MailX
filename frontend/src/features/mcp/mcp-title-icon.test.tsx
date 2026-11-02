import { render, screen, waitFor } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { testClient } from '@/features/accounts/test-fixtures'
import { withTitleBar } from '@/test/title-bar'
import type { McpStatus } from './types'
import { McpTitleIcon } from './mcp-title-icon'
import { useSettingsRequests } from '@/features/settings/settings-requests'

function setup(enabled: boolean, status: McpStatus) {
  const client = testClient()
  if (enabled) client.setMcpEnabled(true)
  client.getMcpStatus.mockResolvedValue(status)
  return client
}

describe('McpTitleIcon', () => {
  it('is not rendered while the server is switched off', async () => {
    const client = setup(false, { state: 'off' })
    render(withTitleBar(<McpTitleIcon client={client} />))
    await waitFor(() => expect(client.getSettings).toHaveBeenCalled())
    expect(screen.queryByRole('button', { name: /MCP/ })).not.toBeInTheDocument()
  })

  it('has no dot while the server is starting', async () => {
    render(withTitleBar(<McpTitleIcon client={setup(true, { state: 'off' })} />))
    expect(await screen.findByRole('button', { name: 'MCP 服务器：正在启动…' })).toBeInTheDocument()
    expect(screen.queryByTestId('mcp-status-dot')).not.toBeInTheDocument()
  })

  it('shows a green dot and the status in the label while running', async () => {
    render(withTitleBar(<McpTitleIcon client={setup(true, { state: 'running', clients: [{ label: 'Cursor' }] })} />))
    const button = await screen.findByRole('button', { name: 'MCP 服务器：运行中，1 个客户端已连接' })
    expect(screen.getByTestId('mcp-status-dot')).toHaveClass('bg-emerald-500')
    await userEvent.hover(button)
    expect(await screen.findAllByText('已连接：Cursor')).not.toHaveLength(0)
  })

  it('says nobody is connected when running without clients', async () => {
    render(withTitleBar(<McpTitleIcon client={setup(true, { state: 'running', clients: [] })} />))
    const button = await screen.findByRole('button', { name: 'MCP 服务器：运行中，暂无客户端连接' })
    await userEvent.hover(button)
    expect(screen.queryByText(/已连接：/)).not.toBeInTheDocument()
  })

  it('lists connected clients, merging repeats and naming unidentified ones', async () => {
    const clients = [{ label: 'Claude Code' }, { label: 'Cursor' }, { label: 'Claude Code' }, { label: null }]
    render(withTitleBar(<McpTitleIcon client={setup(true, { state: 'running', clients })} />))
    await userEvent.hover(await screen.findByRole('button', { name: /MCP 服务器/ }))
    expect(await screen.findAllByText('已连接：Claude Code ×2、Cursor、未知客户端')).not.toHaveLength(0)
  })

  it('shows a red dot and the reason after a failure', async () => {
    render(withTitleBar(<McpTitleIcon client={setup(true, { state: 'failed', code: 'endpointInUse' })} />))
    expect(await screen.findByRole('button', { name: /启动失败：.*已被占用/ })).toBeInTheDocument()
    expect(screen.getByTestId('mcp-status-dot')).toHaveClass('bg-destructive')
  })

  it('asks for the MCP settings page when clicked', async () => {
    const requested = vi.fn()
    function Listener() { useSettingsRequests(requested); return null }
    render(withTitleBar(<><Listener /><McpTitleIcon client={setup(true, { state: 'running', clients: [] })} /></>))
    await userEvent.click(await screen.findByRole('button', { name: /MCP 服务器/ }))
    expect(requested).toHaveBeenCalledWith({ section: 'mcp' })
  })
})
