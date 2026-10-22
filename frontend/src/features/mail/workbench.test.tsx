import { render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { createMockClient } from '@/lib/mock-client'
import { testClient } from '@/features/accounts/test-fixtures'
import { searchFor } from '@/test/search'
import { withTitleBar } from '@/test/title-bar'
import { Workbench } from './workbench'

async function renderWorkbench() {
  render(withTitleBar(<Workbench client={createMockClient()} />))
  const list = await screen.findByRole('list', { name: '邮件列表' })
  return { list }
}

describe('workbench', () => {
  it('opens on the unified inbox across accounts and shows the selected message', async () => {
    const { list } = await renderWorkbench()
    expect(screen.getByRole('heading', { name: '全部收件箱' })).toBeInTheDocument()
    const rows = within(list).getAllByRole('listitem')
    expect(within(list).getAllByText('工作邮箱').length).toBeGreaterThan(0)
    expect(within(list).getAllByText('个人邮箱').length).toBeGreaterThan(0)

    await userEvent.click(rows[0])
    expect(rows[0]).toHaveAttribute('aria-current', 'true')
    expect(await screen.findByRole('heading', { level: 2, name: '第三季度产品路线图评审' })).toBeInTheDocument()
    expect(screen.getByRole('region', { name: '附件' })).toHaveTextContent('Q3 路线图草稿.pdf')
  })

  it('narrows to one folder, filters unread and searches', async () => {
    await renderWorkbench()
    const rows = () => within(screen.getByRole('list', { name: '邮件列表' })).getAllByRole('listitem')
    const sent = screen.getAllByRole('button', { name: '已发送' })[0]
    await userEvent.click(sent)
    expect(screen.getByRole('heading', { name: '已发送' })).toBeInTheDocument()
    await waitFor(() => expect(rows()).toHaveLength(1))

    await userEvent.click(screen.getByRole('button', { name: '全部收件箱' }))
    await userEvent.click(screen.getByRole('tab', { name: '未读' }))
    await waitFor(() => {
      for (const row of rows()) expect(within(row).getByLabelText('未读')).toBeInTheDocument()
    })

    await searchFor('不存在的关键词')
    expect(await screen.findByText('没有匹配的邮件')).toBeInTheDocument()
    expect(screen.queryByRole('list', { name: '邮件列表' })).not.toBeInTheDocument()
  })

  it('opens the settings dialog from the sidebar', async () => {
    await renderWorkbench()
    await userEvent.click(screen.getByRole('button', { name: '设置' }))
    expect(await screen.findByRole('dialog', { name: '设置' })).toBeInTheDocument()
    expect(await screen.findByRole('radiogroup', { name: '颜色模式' })).toBeInTheDocument()
  })

  it('opens the account manager from the sidebar and lists accounts beside the form', async () => {
    await renderWorkbench()
    await userEvent.click(screen.getByRole('button', { name: '账号' }))
    const dialog = await screen.findByRole('dialog', { name: '邮箱账号' })
    expect(await within(dialog).findByLabelText('账号名称')).toBeInTheDocument()
    expect(within(dialog).getAllByRole('button', { name: '添加账号' }).length).toBeGreaterThan(0)
  })

  it('opens the add-account dialog on the method step when there are no accounts', async () => {
    render(withTitleBar(<Workbench client={testClient()} />))
    await userEvent.click(await screen.findByRole('button', { name: '添加邮箱账号' }))
    const dialog = await screen.findByRole('dialog', { name: '添加邮箱账号' })
    expect(within(dialog).queryByLabelText('邮箱地址')).not.toBeInTheDocument()
    await userEvent.click(within(dialog).getByRole('radio', { name: /IMAP\/SMTP/ }))
    await userEvent.click(within(dialog).getByRole('button', { name: '下一步' }))
    expect(within(dialog).getByLabelText('邮箱地址')).toHaveValue('')
  })
})
