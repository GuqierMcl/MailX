import { describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen } from '@/test/render'
import type { ConfirmationRequest } from '@/generated/accounts'
import { ConfirmCard } from './confirm-card'

const SEND: ConfirmationRequest = {
  action: 'send', tool: 'send_email',
  details: {
    type: 'send', accountName: '工作邮箱', accountAddress: 'me@example.com',
    to: [{ name: '王五', address: 'wang@example.com' }], cc: [{ name: null, address: 'cc@example.com' }], bcc: [],
    subject: '周报', bodyPreview: '本周完成了三件事……', attachments: [{ fileName: '周报.pdf', size: 2048 }],
    warnings: ['newRecipient', 'notInConversation', 'hasAttachments'],
  },
}
const ORGANIZE: ConfirmationRequest = {
  action: 'organize', tool: 'archive_emails',
  details: { type: 'organize', change: { type: 'archive' }, count: 7, subjects: ['账单', '通知'] },
}

describe('ConfirmCard', () => {
  it('lists what would be sent and the plain-worded warnings, with 取消 focused first', () => {
    render(<ConfirmCard request={SEND} status="waiting" onRespond={vi.fn()} />)
    const card = screen.getByRole('group', { name: '发送这封邮件？' })
    expect(card).toHaveTextContent('工作邮箱 <me@example.com>')
    expect(card).toHaveTextContent('王五 <wang@example.com>')
    expect(card).toHaveTextContent('cc@example.com')
    expect(card).toHaveTextContent('周报')
    expect(card).toHaveTextContent('本周完成了三件事')
    expect(card).toHaveTextContent('周报.pdf (2 KB)')
    for (const warning of ['第一次联系这个地址', '这次对话里没出现过这个收件人', '带有附件']) expect(card).toHaveTextContent(warning)
    expect(screen.getByRole('button', { name: '取消' })).toHaveFocus()
    expect(screen.getByRole('button', { name: '发送' })).toBeInTheDocument()
  })

  it('names the action and the emails for an organize request', () => {
    render(<ConfirmCard request={ORGANIZE} status="waiting" onRespond={vi.fn()} />)
    const card = screen.getByRole('group', { name: '归档这些邮件？' })
    expect(card).toHaveTextContent('7 封邮件')
    expect(card).toHaveTextContent('账单')
    expect(screen.getByRole('button', { name: '归档' })).toBeInTheDocument()
  })

  it('describes a draft change from the host data', () => {
    render(<ConfirmCard request={{ action: 'userDraft', tool: 'update_draft', details: { type: 'draft', subject: '计划', origin: 'user', change: { type: 'update', fields: ['subject', 'bodyHtml'] } } }} status="waiting" onRespond={vi.fn()} />)
    expect(screen.getByRole('group', { name: '修改这份草稿？' })).toHaveTextContent('修改主题、正文')
  })

  it('answers yes or no once, and gives the focus back', async () => {
    const respond = vi.fn()
    const answered = vi.fn()
    render(<ConfirmCard request={SEND} status="waiting" onRespond={respond} onAnswered={answered} />)
    await userEvent.click(screen.getByRole('button', { name: '发送' }))
    expect(respond).toHaveBeenCalledWith(true)
    expect(answered).toHaveBeenCalledTimes(1)
    respond.mockClear()
    await userEvent.click(screen.getByRole('button', { name: '取消' }))
    expect(respond).toHaveBeenCalledWith(false)
  })

  it('does not take the focus from the field the user is typing in', () => {
    render(<textarea aria-label="field" autoFocus />)
    expect(screen.getByLabelText('field')).toHaveFocus()
    render(<ConfirmCard request={SEND} status="waiting" onRespond={vi.fn()} />)
    expect(screen.getByLabelText('field')).toHaveFocus()
  })

  it('folds to one line once answered', () => {
    const { rerender } = render(<ConfirmCard request={SEND} status="waiting" onRespond={vi.fn()} />)
    rerender(<ConfirmCard request={SEND} status="approved" onRespond={vi.fn()} />)
    expect(screen.getByRole('status')).toHaveTextContent('已同意 · 发送已排队')
    // The details are still in the page for the fold to animate, but unreachable.
    expect(screen.getByRole('button', { name: '取消', hidden: true }).closest('[inert]')).not.toBeNull()
    rerender(<ConfirmCard request={SEND} status="denied" onRespond={vi.fn()} />)
    expect(screen.getByRole('status')).toHaveTextContent('已取消')
    rerender(<ConfirmCard request={SEND} status="closed" onRespond={vi.fn()} />)
    expect(screen.getByRole('status')).toHaveTextContent('已取消')
  })
})
