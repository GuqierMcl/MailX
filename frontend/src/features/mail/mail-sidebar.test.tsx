import { render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it } from 'vitest'
import { createMockClient } from '@/lib/mock-client'
import { Workbench } from './workbench'

async function renderSidebar() {
  render(<Workbench client={createMockClient()} />)
  await screen.findByRole('list', { name: '邮件列表' })
  return screen.getByRole('button', { name: '工作邮箱' })
}

afterEach(() => localStorage.clear())

describe('mail sidebar', () => {
  it('lists each account with its folders nested underneath', async () => {
    await renderSidebar()
    expect(screen.getByRole('button', { name: '个人邮箱' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'MailX' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '华东区' })).toBeInTheDocument()
    expect(screen.getAllByRole('button', { name: '收件箱' })).toHaveLength(2)
  })

  it('collapses an account and remembers it', async () => {
    const account = await renderSidebar()
    await userEvent.click(account)
    expect(screen.queryByRole('button', { name: 'MailX' })).not.toBeInTheDocument()
    expect(JSON.parse(localStorage.getItem('mailx.sidebar.collapsed') ?? '[]')).toHaveLength(1)
  })

  it('shows child unread mail on a collapsed parent folder', async () => {
    await renderSidebar()
    const projects = screen.getByRole('button', { name: '项目' }).closest('li')!
    expect(within(projects).getAllByText('1')).toHaveLength(1)

    await userEvent.click(screen.getByRole('button', { name: '收起项目' }))
    expect(screen.queryByRole('button', { name: 'MailX' })).not.toBeInTheDocument()
    expect(within(projects).getByText('1')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '展开项目' })).toBeInTheDocument()
  })

  it('opens a folder from the tree and treats container folders as toggles only', async () => {
    await renderSidebar()
    await userEvent.click(screen.getByRole('button', { name: 'MailX' }))
    expect(screen.getByRole('heading', { name: 'MailX' })).toBeInTheDocument()

    await userEvent.click(screen.getByRole('button', { name: '客户' }))
    expect(screen.queryByRole('button', { name: '华东区' })).not.toBeInTheDocument()
    expect(screen.getByRole('heading', { name: 'MailX' })).toBeInTheDocument()
  })
})

describe('sidebar unread numbers follow read changes', () => {
  // Both accounts have an inbox, so look inside the work account's own tree.
  const work = () => screen.getByRole('button', { name: '工作邮箱' }).closest('li')!
  const numberIn = (row: HTMLElement) => {
    const text = within(row).queryByText(/^\d+$/)?.textContent
    return text === undefined ? null : Number(text)
  }
  const workInbox = () => within(work()).getByRole('button', { name: '收件箱' }).closest('li')!
  // The account row's own badge is the first number in its subtree.
  const accountNumber = () => Number(work().querySelector('.tabular-nums')?.textContent)

  it('drops the folder and account numbers when a message is read, and restores them when it is unread again', async () => {
    await renderSidebar()
    const before = { folder: numberIn(workInbox()), account: accountNumber() }
    expect(before.folder).toBeGreaterThan(0)

    await userEvent.click(within(work()).getByRole('button', { name: '收件箱' }))
    const unreadRows = (await screen.findAllByRole('listitem')).filter((row) => within(row).queryByLabelText('未读'))
    await userEvent.click(unreadRows[0])
    // Opening marks it read; the sidebar must follow without any sync event.
    await waitFor(() => expect(numberIn(workInbox())).toBe((before.folder ?? 0) - 1 || null))
    expect(accountNumber()).toBe(before.account - 1)

    await userEvent.click(await screen.findByRole('button', { name: '标为未读' }))
    await waitFor(() => expect(numberIn(workInbox())).toBe(before.folder))
    expect(accountNumber()).toBe(before.account)
  })
})
