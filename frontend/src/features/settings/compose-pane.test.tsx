import { describe, expect, it } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen } from '@/test/render'
import { exampleAccount, testClient } from '@/features/accounts/test-fixtures'
import { ComposePane } from './compose-pane'

describe('compose account choices', () => {
  it('uses display order for choices while retaining the saved default sender', async () => {
    const first = exampleAccount()
    const second = exampleAccount({ id: 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', displayName: '个人', emailAddress: 'home@example.com' })
    const client = testClient([first, second])
    await client.updateSettings({ compose: { defaultAccountId: first.id, undoSendSeconds: 0 }, layout: { accountOrder: [second.id, first.id] } })
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
})
