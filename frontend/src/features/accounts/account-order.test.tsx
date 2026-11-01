import { afterEach, describe, expect, it } from 'vitest'
import { act, fireEvent, render, screen, waitFor } from '@/test/render'
import type { MailxClient } from '@/lib/mailx-client'
import { createMockClient } from '@/lib/mock-client'
import { useUpdateLayout } from '@/features/settings/settings-queries'
import { exampleAccount, testClient } from './test-fixtures'
import { sortAccountsForDisplay, useOrderedAccounts } from './account-order'

const first = exampleAccount()
const second = exampleAccount({ id: 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb' })
const third = exampleAccount({ id: 'cccccccc-cccc-4ccc-8ccc-cccccccccccc' })

afterEach(() => localStorage.clear())

describe('account display order', () => {
  it('ignores missing and repeated ids, then appends new accounts in their original order', () => {
    const accounts = Object.freeze([first, second, third])
    const order = Object.freeze(['removed-account', third.id, third.id])
    expect(sortAccountsForDisplay(accounts, order)).toEqual([third, first, second])
    expect(accounts).toEqual([first, second, third])
    expect(order).toEqual(['removed-account', third.id, third.id])
  })

  it('keeps the original order when the preference is empty or belongs to removed accounts', () => {
    const accounts = [third, first, second]
    expect(sortAccountsForDisplay(accounts, [])).toEqual(accounts)
    expect(sortAccountsForDisplay(accounts, ['removed-account'])).toEqual(accounts)
    expect(sortAccountsForDisplay([], [first.id])).toEqual([])
  })
})

function AccountMenu({ client, accounts, name }: { client: MailxClient; accounts: { id: string }[]; name: string }) {
  const ordered = useOrderedAccounts(client, accounts)
  const update = useUpdateLayout(client)
  return <>
    <output aria-label={name}>{ordered.map((account) => account.id).join(',')}</output>
    <button onClick={() => update.mutate({ accountOrder: [...accounts].reverse().map((account) => account.id) })}>{name} reverse</button>
    {update.isError && <span role="alert">Save failed</span>}
  </>
}

describe('ordered account menus', () => {
  it('follows settings changes in two independent window caches without reloading accounts', async () => {
    const main = createMockClient()
    const composer = createMockClient()
    const accounts = await main.listAccounts()
    const initial = accounts.map((account) => account.id).join(',')
    const reversed = [...accounts].reverse().map((account) => account.id).join(',')
    render(<AccountMenu client={main} accounts={accounts} name="main" />)
    render(<AccountMenu client={composer} accounts={accounts} name="composer" />)
    await waitFor(() => expect(screen.getByLabelText('composer')).toHaveTextContent(initial))

    fireEvent.click(screen.getByRole('button', { name: 'main reverse' }))

    await waitFor(() => expect(screen.getByLabelText('main')).toHaveTextContent(reversed))
    await waitFor(() => expect(screen.getByLabelText('composer')).toHaveTextContent(reversed))
    expect(await main.listAccounts()).toEqual(accounts)
  })

  it('rolls back the optimistic order after a rejected save and submits only that layout field', async () => {
    const accounts = [first, second]
    const client = testClient(accounts)
    let rejectSave: (reason: Error) => void = () => undefined
    client.updateSettings.mockImplementationOnce(() => new Promise((_resolve, reject) => { rejectSave = reject }))
    render(<AccountMenu client={client} accounts={accounts} name="accounts" />)
    await waitFor(() => expect(client.getSettings).toHaveBeenCalled())
    await waitFor(() => expect(screen.getByLabelText('accounts')).toHaveTextContent(`${first.id},${second.id}`))

    fireEvent.click(screen.getByRole('button', { name: 'accounts reverse' }))
    await waitFor(() => expect(screen.getByLabelText('accounts')).toHaveTextContent(`${second.id},${first.id}`))
    expect(client.updateSettings).toHaveBeenCalledWith({ layout: { accountOrder: [second.id, first.id] } })
    await act(async () => { rejectSave(new Error('disk full')) })

    expect(await screen.findByRole('alert')).toHaveTextContent('Save failed')
    expect(screen.getByLabelText('accounts')).toHaveTextContent(`${first.id},${second.id}`)
  })
})
