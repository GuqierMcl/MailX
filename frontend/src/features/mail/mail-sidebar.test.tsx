import { render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it } from 'vitest'
import { createMockClient } from '@/lib/mock-client'
import { Workbench } from './workbench'
import { expandAccount } from '@/test/sidebar'
import { withTitleBar } from '@/test/title-bar'

async function renderSidebar() {
  render(withTitleBar(<Workbench client={createMockClient()} />))
  await screen.findByRole('list', { name: '邮件列表' })
  return within(screen.getByRole('navigation', { name: '邮箱导航' }))
}

/** The top row with this label (the first match: account trees come later). */
const topRow = (nav: ReturnType<typeof within>, name: string) => nav.getAllByRole('button', { name })[0].closest('li')!
const accountRow = (nav: ReturnType<typeof within>, name: string) => nav.getByRole('button', { name }).closest('li')!

afterEach(() => localStorage.clear())

describe('mail sidebar', () => {
  it('shows views across accounts at the top and every folder in each account, collapsed by default', async () => {
    const nav = await renderSidebar()
    for (const name of ['收件箱', '星标', '草稿', '已发送', '更多']) expect(nav.getByRole('button', { name })).toBeInTheDocument()
    expect(nav.getByText('邮箱')).toBeInTheDocument()
    expect(nav.getByRole('button', { name: '工作邮箱' })).toHaveAttribute('aria-expanded', 'false')
    expect(nav.queryByRole('button', { name: 'MailX' })).not.toBeInTheDocument()

    await expandAccount('工作邮箱')
    const work = accountRow(nav, '工作邮箱')
    for (const name of ['收件箱', '已归档', '已发送', '垃圾邮件', '已删除', 'MailX', '华东区']) expect(within(work).getByRole('button', { name })).toBeInTheDocument()
    // The server's own archive and drafts folders keep their names: the top's 归档 and 草稿 are local.
    expect(within(work).getByRole('button', { name: '归档' })).toBeInTheDocument()
    expect(within(work).getByRole('button', { name: '草稿' })).toBeInTheDocument()
    expect(JSON.parse(localStorage.getItem('mailx.sidebar.expanded') ?? '[]')).toEqual([expect.stringMatching(/^account:/)])
  })

  it('opens the account of a folder chosen elsewhere, without remembering it', async () => {
    const nav = await renderSidebar()
    await userEvent.click(screen.getByRole('button', { name: '搜索邮件' }))
    await userEvent.click(await screen.findByRole('option', { name: '个人邮箱 · 旅行' }))
    expect(await screen.findByRole('heading', { name: '旅行' })).toBeInTheDocument()
    expect(within(accountRow(nav, '个人邮箱')).getByRole('button', { name: '旅行' })).toHaveAttribute('data-active')
    expect(localStorage.getItem('mailx.sidebar.expanded')).toBeNull()
    // Closing it is the user's choice and sticks.
    await userEvent.click(nav.getByRole('button', { name: '个人邮箱' }))
    expect(nav.queryByRole('button', { name: '旅行' })).not.toBeInTheDocument()
  })

  it('keeps archive, junk and trash under 更多, opening it on demand and remembering it', async () => {
    const nav = await renderSidebar()
    expect(nav.queryByRole('button', { name: '垃圾邮件' })).not.toBeInTheDocument()
    await userEvent.click(nav.getByRole('button', { name: '更多' }))
    expect(nav.getByRole('button', { name: '垃圾邮件' })).toBeInTheDocument()
    expect(nav.getByRole('button', { name: '已删除' })).toBeInTheDocument()
    expect(JSON.parse(localStorage.getItem('mailx.sidebar.expanded') ?? '[]')).toContain('more')

    await userEvent.click(nav.getByRole('button', { name: '已删除' }))
    expect(screen.getByRole('heading', { name: '全部已删除' })).toBeInTheDocument()
    await userEvent.click(nav.getByRole('button', { name: '更多' }))
    expect(nav.queryByRole('button', { name: '已删除' })).not.toBeInTheDocument()
    expect(JSON.parse(localStorage.getItem('mailx.sidebar.expanded') ?? '[]')).not.toContain('more')
    // A view under 更多 chosen elsewhere opens it again.
    await userEvent.click(screen.getByRole('button', { name: '搜索邮件' }))
    await userEvent.click(await screen.findByRole('option', { name: '全部垃圾邮件' }))
    expect(await nav.findByRole('button', { name: '垃圾邮件' })).toBeInTheDocument()
  })

  it('lists each account under a top row and opens that account folder', async () => {
    const nav = await renderSidebar()
    const inbox = topRow(nav, '收件箱')
    expect(within(inbox).queryByRole('button', { name: '工作邮箱' })).not.toBeInTheDocument()
    await userEvent.click(within(inbox).getByRole('button', { name: '展开收件箱的各账号' }))
    await userEvent.click(within(inbox).getByRole('button', { name: '工作邮箱' }))
    expect(screen.getByRole('heading', { name: '收件箱' })).toBeInTheDocument()
    expect(within(inbox).getByRole('button', { name: '收起收件箱的各账号' })).toBeInTheDocument()

    await userEvent.click(within(inbox).getByRole('button', { name: '收件箱' }))
    expect(screen.getByRole('heading', { name: '全部收件箱' })).toBeInTheDocument()
  })

  it('shows starred mail across accounts, archived mail included and junk or trash left out', async () => {
    const nav = await renderSidebar()
    await userEvent.click(nav.getByRole('button', { name: '星标' }))
    expect(screen.getByRole('heading', { name: '星标' })).toBeInTheDocument()
    const rows = within(screen.getByRole('list', { name: '邮件列表' })).getAllByRole('listitem')
    expect(rows.length).toBeGreaterThan(0)
    for (const row of rows) expect(within(row).getByLabelText('已加星标')).toBeInTheDocument()
  })

  it('collapses an opened account again and remembers it', async () => {
    const nav = await renderSidebar()
    await expandAccount('工作邮箱')
    await userEvent.click(nav.getByRole('button', { name: '工作邮箱' }))
    expect(nav.queryByRole('button', { name: 'MailX' })).not.toBeInTheDocument()
    expect(JSON.parse(localStorage.getItem('mailx.sidebar.expanded') ?? '[]')).toEqual([])
  })

  it('shows child unread mail on a collapsed parent folder', async () => {
    const nav = await renderSidebar()
    await expandAccount('工作邮箱')
    const projects = nav.getByRole('button', { name: '项目' }).closest('li')!
    expect(within(projects).getAllByText('1')).toHaveLength(1)

    await userEvent.click(nav.getByRole('button', { name: '收起项目' }))
    expect(nav.queryByRole('button', { name: 'MailX' })).not.toBeInTheDocument()
    expect(within(projects).getByText('1')).toBeInTheDocument()
    expect(nav.getByRole('button', { name: '展开项目' })).toBeInTheDocument()
  })

  it('opens a folder from the tree and treats container folders as toggles only', async () => {
    const nav = await renderSidebar()
    await expandAccount('工作邮箱')
    await userEvent.click(nav.getByRole('button', { name: 'MailX' }))
    expect(screen.getByRole('heading', { name: 'MailX' })).toBeInTheDocument()

    await userEvent.click(nav.getByRole('button', { name: '客户' }))
    expect(nav.queryByRole('button', { name: '华东区' })).not.toBeInTheDocument()
    expect(screen.getByRole('heading', { name: 'MailX' })).toBeInTheDocument()
  })
})

describe('sidebar unread numbers follow read changes', () => {
  // A row's own number comes before those of the accounts listed below it.
  const numberIn = (row: HTMLElement) => {
    const text = within(row).queryAllByText(/^\d+$/)[0]?.textContent
    return text === undefined ? null : Number(text)
  }

  it('drops the inbox numbers when a message is read, and restores them when it is unread again', async () => {
    const nav = await renderSidebar()
    const inbox = () => topRow(nav, '收件箱')
    await userEvent.click(within(inbox()).getByRole('button', { name: '展开收件箱的各账号' }))
    const workInbox = () => within(inbox()).getByRole('button', { name: '工作邮箱' }).closest('li')!
    const before = { all: numberIn(inbox()), work: numberIn(workInbox()) }
    expect(before.work).toBeGreaterThan(0)

    await userEvent.click(within(inbox()).getByRole('button', { name: '工作邮箱' }))
    const unreadRows = (await screen.findAllByRole('listitem')).filter((row) => within(row).queryByLabelText('未读'))
    await userEvent.click(unreadRows[0])
    // Opening marks it read; the sidebar must follow without any sync event.
    await waitFor(() => expect(numberIn(workInbox())).toBe((before.work ?? 0) - 1 || null))
    expect(numberIn(inbox())).toBe((before.all ?? 0) - 1 || null)

    await userEvent.click(within(screen.getByRole('toolbar', { name: '邮件操作' })).getByRole('button', { name: '标为未读' }))
    await waitFor(() => expect(numberIn(workInbox())).toBe(before.work))
    expect(numberIn(inbox())).toBe(before.all)
  })

  it('lists locally archived mail under 归档, per account too, and keeps it out of the inbox', async () => {
    const nav = await renderSidebar()
    await userEvent.click(nav.getByRole('button', { name: '更多' }))
    const archive = () => topRow(nav, '归档')
    await userEvent.click(within(archive()).getByRole('button', { name: '展开归档的各账号' }))

    await userEvent.click(nav.getByRole('button', { name: '收件箱' }))
    const rows = () => within(screen.getByRole('list', { name: '邮件列表' })).getAllByRole('listitem')
    const before = rows().length
    await userEvent.click(rows()[0])
    const archiveButton = () => within(screen.getByRole('toolbar', { name: '邮件操作' })).getByRole('button', { name: '归档' })
    await waitFor(() => expect(archiveButton()).toBeEnabled())
    await userEvent.click(archiveButton())
    await waitFor(() => expect(rows()).toHaveLength(before - 1))

    await userEvent.click(within(archive()).getByRole('button', { name: '工作邮箱' }))
    expect(screen.getByRole('heading', { name: '已归档' })).toBeInTheDocument()
    await waitFor(() => expect(rows()).toHaveLength(1))
    expect(within(rows()[0]).getByText('已归档')).toBeInTheDocument()
    await userEvent.click(rows()[0])
    expect(within(screen.getByRole('toolbar', { name: '邮件操作' })).getByRole('button', { name: '移回收件箱' })).toBeInTheDocument()

    await userEvent.click(within(archive()).getByRole('button', { name: '归档' }))
    expect(screen.getByRole('heading', { name: '全部已归档' })).toBeInTheDocument()
  })
})
