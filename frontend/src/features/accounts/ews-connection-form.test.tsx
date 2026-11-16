import { act, fireEvent, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import type { EwsSettingsDiscovery } from '@/generated/accounts'
import { EwsConnectionForm } from './ews-connection-form'
import { ConnectionForm } from './connection-form'
import { AddAccountDialog } from './add-account-dialog'
import { exampleAccount, testClient } from './test-fixtures'

const ews = { url: 'https://mail.example.com/EWS/Exchange.asmx', username: 'EXAMPLE\\alice' }
const found: EwsSettingsDiscovery = { status: 'found', endpoint: ews, source: 'https://autodiscover.example.com/autodiscover/autodiscover.xml', targetGrant: null }
const props = () => ({ submitLabel: '下一步', submittingLabel: '保存中…', onBusyChange: vi.fn(), onDirtyChange: vi.fn(), onSubmit: vi.fn() })

function address(value = 'alice@example.com') { fireEvent.change(screen.getByLabelText('邮箱地址'), { target: { value } }) }

describe('independent self-hosted Exchange setup', () => {
  it('keeps the original IMAP/SMTP path manual and never starts Exchange discovery', async () => {
    const client = testClient()
    render(<AddAccountDialog client={client} open accounts={[]} onClose={vi.fn()} onCreated={vi.fn()} />)
    const dialog = screen.getByRole('dialog', { name: '添加邮箱账号' })
    await userEvent.click(within(dialog).getByRole('radio', { name: 'IMAP/SMTP' }))
    await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
    expect(screen.getByLabelText('IMAP 主机')).toBeInTheDocument()
    expect(screen.getByLabelText('SMTP 主机')).toBeInTheDocument()
    address()
    expect(client.discoverEwsSettings).not.toHaveBeenCalled()
    expect(screen.queryByLabelText('Exchange 服务器 URL')).not.toBeInTheDocument()
  })

  it('adds Exchange through its own entry without fabricated IMAP/SMTP endpoints', async () => {
    const client = testClient()
    const onCreated = vi.fn()
    render(<AddAccountDialog client={client} open accounts={[]} onClose={vi.fn()} onCreated={onCreated} />)
    await userEvent.click(screen.getByRole('radio', { name: 'Exchange（自建服务器）' }))
    await userEvent.click(screen.getByRole('button', { name: '下一步' }))
    address()
    await userEvent.click(screen.getByRole('button', { name: '手动填写服务器 URL' }))
    fireEvent.change(screen.getByLabelText('Exchange 服务器 URL'), { target: { value: ews.url } })
    fireEvent.change(screen.getByLabelText('登录名'), { target: { value: ews.username } })
    fireEvent.change(screen.getByLabelText('Exchange 密码'), { target: { value: 'synthetic-secret' } })
    await userEvent.click(screen.getByRole('button', { name: '下一步' }))
    await userEvent.click(screen.getByRole('button', { name: '完成' }))
    await waitFor(() => expect(onCreated).toHaveBeenCalledOnce())
    const config = client.createAccount.mock.calls[0][0].config
    expect(config).toMatchObject({ backend: 'ews', ews, providerId: 'exchange-self-hosted' })
    expect(config).not.toHaveProperty('imap')
    expect(config).not.toHaveProperty('smtp')
    expect(client.discoverEwsSettings).not.toHaveBeenCalled()
  })

  it('keeps AuthRequired on the same form and sends credentials only after exact-target consent', async () => {
    const client = testClient()
    const source = 'https://autodiscover.example.com/autodiscover/autodiscover.xml'
    client.discoverEwsSettings.mockResolvedValueOnce({ status: 'authRequired', source, endpoint: null, targetGrant: 'host-grant' }).mockResolvedValueOnce(found)
    render(<EwsConnectionForm {...props()} account={null} client={client} />)
    address()
    await userEvent.click(screen.getByRole('button', { name: '查找服务器' }))
    expect(client.discoverEwsSettings).toHaveBeenCalledWith({ emailAddress: 'alice@example.com', route: { kind: 'direct' }, username: null, password: null, targetGrant: null })
    expect(await screen.findByText(`仅向 ${source} 发送此处凭据，以查找邮箱设置。`)).toBeInTheDocument()
    fireEvent.change(screen.getByLabelText('Exchange 密码'), { target: { value: 'synthetic-secret' } })
    expect(screen.getByLabelText('登录名')).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '向此服务器继续查找' }))
    await waitFor(() => expect(client.discoverEwsSettings).toHaveBeenCalledTimes(2))
    expect(client.discoverEwsSettings.mock.calls[1][0]).toMatchObject({ targetGrant: 'host-grant', username: 'alice@example.com', password: 'synthetic-secret' })
    expect(await screen.findByLabelText('Exchange 服务器 URL')).toHaveValue(ews.url)
    expect(screen.getByLabelText('登录名')).toHaveValue(ews.username)
  })

  it('discards a late lookup after the email address is edited', async () => {
    const client = testClient()
    let release!: (result: EwsSettingsDiscovery) => void
    client.discoverEwsSettings.mockReturnValue(new Promise((resolve) => { release = resolve }))
    render(<EwsConnectionForm {...props()} account={null} client={client} />)
    address()
    await userEvent.click(screen.getByRole('button', { name: '查找服务器' }))
    address('other@example.com')
    await act(async () => { release(found) })
    expect(screen.queryByLabelText('Exchange 服务器 URL')).not.toBeInTheDocument()
    expect(screen.queryByText('已找到服务器，请核对 URL 和登录名后继续。')).not.toBeInTheDocument()
  })

  it('rejects incomplete custom proxies before any discovery request', async () => {
    const client = testClient()
    render(<EwsConnectionForm {...props()} account={null} client={client} />)
    address()
    await userEvent.click(screen.getByRole('combobox', { name: '连接方式' }))
    await userEvent.click(await screen.findByRole('option', { name: '自定义代理' }))
    await userEvent.click(screen.getByRole('button', { name: '查找服务器' }))
    expect(client.discoverEwsSettings).not.toHaveBeenCalled()
    expect(screen.getByLabelText('代理主机')).toHaveAttribute('aria-invalid', 'true')
  })

  it('preserves the Exchange endpoint when saving a route-only edit', async () => {
    const client = testClient()
    const base = exampleAccount()
    const { imap: _imap, smtp: _smtp, ...common } = base
    void _imap; void _smtp
    const account = { ...common, backend: 'ews' as const, ews, authentication: { kind: 'ewsNtlm' as const, credential: base.authentication.imap } }
    const onSubmit = vi.fn()
    render(<EwsConnectionForm {...props()} account={account} client={client} onSubmit={onSubmit} />)
    await userEvent.click(screen.getByRole('combobox', { name: '连接方式' }))
    await userEvent.click(await screen.findByRole('option', { name: '使用应用默认' }))
    expect(screen.getByLabelText('Exchange 服务器 URL')).toHaveValue(ews.url)
    await userEvent.click(screen.getByRole('button', { name: '下一步' }))
    await waitFor(() => expect(onSubmit).toHaveBeenCalledOnce())
    expect(onSubmit.mock.calls[0][0]).toMatchObject({ backend: 'ews', ews, route: { kind: 'inherit' }, authentication: { kind: 'password', mode: 'shared', password: { action: 'keep' } } })
    expect(onSubmit.mock.calls[0][0]).not.toHaveProperty('imap')
    expect(client.discoverEwsSettings).not.toHaveBeenCalled()
  })

  it('keeps automatic discovery out of the original connection component', () => {
    const client = testClient()
    render(<ConnectionForm {...props()} account={null} intro={{ title: '说明', description: '说明' }} />)
    address()
    expect(client.discoverEwsSettings).not.toHaveBeenCalled()
    expect(screen.getByLabelText('IMAP 主机')).toBeInTheDocument()
  })
})
