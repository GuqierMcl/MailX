import { describe, expect, it } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor } from '@/test/render'
import { exampleAccount, testClient } from '@/features/accounts/test-fixtures'
import { ComposePane } from './compose-pane'

describe('compose account choices', () => {
  it('uses display order for choices while retaining the saved default sender', async () => {
    const first = exampleAccount()
    const second = exampleAccount({ id: 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', displayName: '个人', emailAddress: 'home@example.com' })
    const client = testClient([first, second])
    await client.updateSettings({ compose: { defaultAccountId: first.id, undoSendSeconds: 0, openMode: 'window' }, layout: { accountOrder: [second.id, first.id] } })
    client.updateSettings.mockClear()
    render(<ComposePane client={client} />)
    const sender = await screen.findByRole('combobox', { name: '默认发件账号' })
    expect(sender).toHaveTextContent(first.emailAddress)
    await userEvent.click(sender)
    const options = await screen.findAllByRole('option')
    expect(options).toHaveLength(3)
    expect(options[1]).toHaveTextContent(second.emailAddress)
    expect(options[2]).toHaveTextContent(first.emailAddress)
    expect(client.updateSettings).not.toHaveBeenCalled()
  })

  it('offers where the composer opens and saves the choice', async () => {
    const client = testClient([exampleAccount()])
    render(<ComposePane client={client} />)
    const mode = await screen.findByRole('combobox', { name: '写信打开方式' })
    expect(mode).toHaveTextContent('新窗口')
    await userEvent.click(mode)
    const options = await screen.findAllByRole('option')
    expect(options.map((option) => option.textContent)).toEqual(['新窗口', '主窗口内'])
    await userEvent.click(screen.getByRole('option', { name: '主窗口内' }))
    await waitFor(() => expect(client.updateSettings).toHaveBeenCalledWith({ compose: expect.objectContaining({ openMode: 'inline' }) }))
    expect(await screen.findByRole('combobox', { name: '写信打开方式' })).toHaveTextContent('主窗口内')
  })
})
