import { fireEvent, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { AccountsPage } from './accounts-page'
import { exampleAccount, testClient } from './test-fixtures'

describe('account workspace', () => {
  it('explicitly disables browser persistence without calling the client', () => {
    const client = { ...testClient(), available: false }
    render(<AccountsPage client={client} />)
    expect(screen.getByText('桌面能力不可用')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '添加账号' })).toBeDisabled()
    expect(client.listAccounts).not.toHaveBeenCalled()
    expect(client.createAccount).not.toHaveBeenCalled()
    expect(screen.queryByLabelText('邮箱地址')).not.toBeInTheDocument()
  })

  it('shows the empty state then opens a usable new-account form', async () => {
    render(<AccountsPage client={testClient()} />)
    expect(screen.getByRole('status', { name: '正在加载账号配置' })).toBeInTheDocument()
    await userEvent.click(await screen.findByRole('button', { name: '添加第一个账号' }))
    expect(screen.getByRole('heading', { name: '添加邮箱账号' })).toBeInTheDocument()
    expect(screen.getByLabelText('邮箱地址')).toHaveValue('')
    expect(screen.queryByRole('button', { name: /OAuth|连接测试|删除/ })).not.toBeInTheDocument()
  })

  it('loads account detail through get and protects dirty fields when switching accounts', async () => {
    const first = exampleAccount()
    const second = exampleAccount({ id: 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', displayName: '个人邮箱', emailAddress: 'personal@example.com' })
    const client = testClient([first, second])
    render(<AccountsPage client={client} />)
    expect(await screen.findByDisplayValue('work@example.com')).toBeInTheDocument()
    expect(client.getAccount).toHaveBeenCalledWith(first.id)
    fireEvent.change(screen.getByLabelText('账号名称'), { target: { value: '未保存的名字' } })
    await userEvent.click(screen.getByRole('button', { name: /个人邮箱 personal@example.com/ }))
    expect(await screen.findByRole('alertdialog')).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '继续编辑' }))
    expect(screen.getByLabelText('账号名称')).toHaveValue('未保存的名字')
    expect(client.getAccount).toHaveBeenCalledTimes(1)
    await userEvent.click(screen.getByRole('button', { name: /个人邮箱 personal@example.com/ }))
    await userEvent.click(screen.getByRole('button', { name: '放弃修改' }))
    await waitFor(() => expect(screen.getByLabelText('邮箱地址')).toHaveValue('personal@example.com'))
    expect(client.getAccount).toHaveBeenCalledWith(second.id)
  })
})
