import { act, render, screen, waitFor } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import type { AccountSyncState, Folder } from '@/generated/accounts'
import { exampleAccount, exampleFolderSyncState, exampleSyncState, testClient } from '@/features/accounts/test-fixtures'
import { Workbench } from './workbench'

const account = exampleAccount()

function folder(id: string, overrides: Partial<Folder> = {}): Folder {
  return { id, accountId: account.id, parentId: null, name: id, role: 'other', selectable: true, countsKnown: true, unreadCount: 0, totalCount: 0, ...overrides }
}

function desktopLike(folders: Folder[] = [], states: AccountSyncState[] = []) {
  const client = testClient([account])
  let emit: (state: AccountSyncState) => void = () => undefined
  client.listFolders.mockResolvedValue(folders)
  client.listSyncStates.mockResolvedValue(states)
  client.listEmails.mockResolvedValue({ items: [], nextCursor: null, coverage: { headersComplete: false, bodiesComplete: false, syncedAfter: null, syncedBefore: null, windowCapped: false } })
  client.onSyncState.mockImplementation((listener) => {
    emit = listener
    return () => undefined
  })
  return { client, emit: (state: AccountSyncState) => act(() => emit(state)) }
}

const failed: AccountSyncState = exampleSyncState({
  accountId: account.id,
  phase: 'failed',
  lastSyncedAt: null,
  failure: { code: 'authentication', message: '服务器拒绝了登录，请检查用户名和密码或授权码。', detail: 'login rejected: Invalid credentials', willRetry: false },
})

describe('folder sync in the workbench', () => {
  it('shows server folders under their system names', async () => {
    const { client } = desktopLike([folder('INBOX', { role: 'inbox' }), folder('Sent Messages', { role: 'sent' }), folder('Receipts')])
    render(<Workbench client={client} />)
    expect(await screen.findByRole('button', { name: '已发送' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '收件箱' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Receipts' })).toBeInTheDocument()
    expect(screen.queryByText('Sent Messages')).not.toBeInTheDocument()
  })

  it('says folders are syncing before the first listing arrives, then refetches when it does', async () => {
    const { client, emit } = desktopLike()
    render(<Workbench client={client} />)
    expect(await screen.findByText('正在同步文件夹…')).toBeInTheDocument()

    client.listFolders.mockResolvedValue([folder('INBOX', { role: 'inbox', unreadCount: 2 })])
    emit(exampleSyncState({ folderRevision: 1, folders: [exampleFolderSyncState('INBOX')] }))
    expect(await screen.findByRole('button', { name: '收件箱' })).toBeInTheDocument()
    expect(screen.queryByText('正在同步文件夹…')).not.toBeInTheDocument()
  })

  it('explains a failed sync on the account row', async () => {
    const { client } = desktopLike([], [failed])
    render(<Workbench client={client} />)
    const action = await screen.findByRole('button', { name: /^同步失败：邮箱服务器拒绝了账号登录/ })
    expect(screen.getByText('暂无文件夹')).toBeInTheDocument()
    await userEvent.hover(action)
    expect(await screen.findByText('login rejected: Invalid credentials', {}, { timeout: 3000 })).toBeInTheDocument()
    expect(screen.getByText('修改账号设置后会重新尝试。')).toBeInTheDocument()
  })

  it('tells the user inbox headers are syncing', async () => {
    const { client } = desktopLike([folder('INBOX', { role: 'inbox' })], [exampleSyncState({ phase: 'syncing', folders: [exampleFolderSyncState('INBOX', { phase: 'syncing' })] })])
    render(<Workbench client={client} />)
    expect(await screen.findByText('正在同步邮件…')).toBeInTheDocument()
    await waitFor(() => expect(client.listEmails).toHaveBeenCalled())
  })

  it('shows unknown counts instead of a verified zero', async () => {
    const { client } = desktopLike([folder('INBOX', { role: 'inbox', countsKnown: false }), folder('Unknown', { countsKnown: false })])
    render(<Workbench client={client} />)
    await screen.findByRole('button', { name: 'Unknown' })
    const count = screen.getByRole('button', { name: 'Unknown' }).closest('li')!.querySelector('[aria-label="未读数待刷新"]')
    expect(count).toHaveTextContent('—')
    expect(count).not.toHaveTextContent('0')
  })
})
