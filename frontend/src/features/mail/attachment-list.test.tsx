import { act, render, screen, waitFor } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { testClient } from '@/features/accounts/test-fixtures'
import type { AttachmentSummary } from '@/generated/accounts'
import { MailxClientError } from '@/lib/mailx-client'
import { AttachmentList } from './attachment-list'
import { i18n } from '@/i18n'

const report: AttachmentSummary = {
  id: 'mail-1:2', filename: '季度报告.pdf', mimeType: 'application/pdf', size: 1_258_291, isInline: false,
}
const notes: AttachmentSummary = { id: 'mail-1:3', filename: 'notes.txt', mimeType: 'text/plain', size: 42, isInline: false }

function setup(save?: (client: ReturnType<typeof testClient>) => void) {
  const client = testClient()
  save?.(client)
  render(<AttachmentList client={client} emailId="mail-1" attachments={[report, notes]} />)
  return client
}

afterEach(() => vi.useRealTimers())

describe('attachment list', () => {
  it('switches existing labels and failures without changing attachment data or saving again', async () => {
    const client = setup((mock) => mock.saveAttachment.mockRejectedValue(new MailxClientError({
      code: 'storageUnavailable', field: null, message: 'legacy message must not be shown',
    })))
    await userEvent.click(screen.getByRole('button', { name: '保存附件 notes.txt' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('无法读写本地数据')
    await act(async () => { await i18n.changeLanguage('en') })
    expect(screen.getByText('2 attachments')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Save attachment notes.txt' })).toBeEnabled()
    expect(screen.getByRole('alert')).toHaveTextContent('Cannot access local data')
    expect(screen.getByText('季度报告.pdf')).toBeInTheDocument()
    expect(screen.getByText('1.2 MB')).toBeInTheDocument()
    expect(screen.queryByText('legacy message must not be shown')).not.toBeInTheDocument()
    expect(client.saveAttachment).toHaveBeenCalledTimes(1)
  })

  it('lists name and size without the sender-claimed type', () => {
    setup()
    expect(screen.getByText('2 个附件')).toBeInTheDocument()
    expect(screen.getByText('季度报告.pdf')).toBeInTheDocument()
    expect(screen.getByText('1.2 MB')).toBeInTheDocument()
    expect(screen.queryByText('application/pdf')).not.toBeInTheDocument()
  })

  it('asks the host to save exactly the attachment that was clicked, with no path', async () => {
    const client = setup()
    await userEvent.click(screen.getByRole('button', { name: '保存附件 notes.txt' }))
    expect(client.saveAttachment).toHaveBeenCalledTimes(1)
    expect(client.saveAttachment).toHaveBeenCalledWith('mail-1', 'mail-1:3')
    expect(await screen.findByRole('status')).toHaveTextContent('已保存')
  })

  it('shows progress and ignores a second click while saving', async () => {
    let finish: (value: 'saved') => void = () => undefined
    const client = setup((c) => c.saveAttachment.mockReturnValue(new Promise((resolve) => { finish = resolve })))
    const button = screen.getByRole('button', { name: '保存附件 季度报告.pdf' })
    await userEvent.click(button)
    expect(await screen.findByRole('status', { name: '正在保存' })).toBeInTheDocument()
    expect(button).toBeDisabled()
    await userEvent.click(button)
    expect(client.saveAttachment).toHaveBeenCalledTimes(1)
    finish('saved')
    await waitFor(() => expect(button).toBeEnabled())
  })

  it('treats closing the dialog as neither success nor failure', async () => {
    setup((c) => c.saveAttachment.mockResolvedValue('cancelled'))
    await userEvent.click(screen.getByRole('button', { name: '保存附件 季度报告.pdf' }))
    await waitFor(() => expect(screen.getByRole('button', { name: '保存附件 季度报告.pdf' })).toBeEnabled())
    expect(screen.queryByRole('status')).not.toBeInTheDocument()
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
  })

  it('shows the fixed failure message and allows trying again', async () => {
    const client = setup((c) => c.saveAttachment.mockRejectedValueOnce(
      new MailxClientError({ code: 'storageUnavailable', message: '无法读写本地数据，请检查可用空间与访问权限。', field: null }),
    ))
    const button = screen.getByRole('button', { name: '保存附件 季度报告.pdf' })
    await userEvent.click(button)
    expect(await screen.findByRole('alert')).toHaveTextContent('无法读写本地数据')
    await userEvent.click(button)
    expect(client.saveAttachment).toHaveBeenCalledTimes(2)
    expect(await screen.findByRole('status')).toHaveTextContent('已保存')
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
  })

  it('never shows raw transport errors', async () => {
    setup((c) => c.saveAttachment.mockRejectedValue(new Error('C:\\Users\\secret\\path failed')))
    await userEvent.click(screen.getByRole('button', { name: '保存附件 季度报告.pdf' }))
    const alert = await screen.findByRole('alert')
    expect(alert).not.toHaveTextContent('secret')
    expect(alert).toHaveTextContent('通信中断')
  })

  it('clears the saved note after a moment', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true })
    setup()
    await userEvent.click(screen.getByRole('button', { name: '保存附件 notes.txt' }))
    expect(await screen.findByRole('status')).toHaveTextContent('已保存')
    await act(async () => { await vi.advanceTimersByTimeAsync(3000) })
    expect(screen.queryByRole('status')).not.toBeInTheDocument()
  })
})
