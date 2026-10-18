import { render, screen, within } from '@testing-library/react'
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
