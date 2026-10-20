import { act, fireEvent, render, screen, waitFor } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { testClient } from '@/features/accounts/test-fixtures'
import type { BodyLinkRequest } from '@/lib/mailx-client'
import { BodyDocument } from './body-document'
import { EmailView } from './email-view'
import type { EmailDetail } from '@/generated/accounts'

vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, convertFileSrc: () => 'mailx-body://localhost/' }))
const url = 'https://example.com/a/long/path?destination=complete&x=1#fragment'
function setup() {
  const client = testClient()
  let serial = 0
  client.prepareBody.mockImplementation(async () => ({ documentToken: `token-${++serial}` }))
  const listeners = new Set<(event: BodyLinkRequest) => void>()
  client.onLinkRequest.mockImplementation((listener) => { listeners.add(listener); return () => { listeners.delete(listener) } })
  const emit = (documentToken: string, requestId = 'request-1', emailId = 'one') => act(() => {
    listeners.forEach((listener) => listener({ emailId, documentToken, requestId, index: 0, url }))
  })
  return { client, emit }
}
async function frame() {
  const frame = await screen.findByTitle('邮件 HTML 正文')
  fireEvent.load(frame)
  return frame
}
afterEach(() => { vi.restoreAllMocks() })

describe('isolated body and confirmed links', () => {
  it('uses only a token-bound protocol URL and the empty sandbox, displays full URL, cancel opens nothing, confirm opens once', async () => {
    const { client, emit } = setup()
    render(<BodyDocument client={client} emailId="one" />)
    const iframe = await frame()
    expect(iframe).toHaveAttribute('src', 'mailx-body://localhost/document/one?token=token-1')
    expect(iframe).toHaveAttribute('sandbox', '')
    expect(iframe).toHaveAttribute('referrerpolicy', 'no-referrer')
    expect(iframe).not.toHaveAttribute('srcdoc')
    emit('token-1')
    expect(await screen.findByRole('alertdialog')).toHaveTextContent(url)
    await userEvent.click(screen.getByRole('button', { name: '取消' }))
    expect(client.openLink).not.toHaveBeenCalled()
    emit('token-1', 'request-2')
    const confirm = await screen.findByRole('button', { name: '确认打开' })
    fireEvent.click(confirm)
    fireEvent.click(confirm)
    expect(client.openLink).toHaveBeenCalledTimes(1)
    expect(client.openLink).toHaveBeenCalledWith({ emailId: 'one', index: 0, documentToken: 'token-1', requestId: 'request-2' })
  })

  it('rejects A → B → A events, reloads, and events after visibility changes', async () => {
    const { client, emit } = setup()
    const { rerender } = render(<BodyDocument client={client} emailId="one" />)
    await frame()
    emit('token-1')
    rerender(<BodyDocument client={client} emailId="two" />)
    await frame()
    rerender(<BodyDocument client={client} emailId="one" />)
    await frame()
    emit('token-1', 'late')
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    emit('token-3', 'current')
    expect(await screen.findByRole('alertdialog')).toBeInTheDocument()
    vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden')
    fireEvent(document, new Event('visibilitychange'))
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('visible')
    fireEvent(document, new Event('visibilitychange'))
    await frame()
    emit('token-3', 'hidden-late')
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '重新加载正文' }))
    await frame()
    emit('token-4', 'reload-late')
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    expect(client.openLink).not.toHaveBeenCalled()
  })

  it('requests numbered links without accepting arbitrary URLs and ignores requests before loading', async () => {
    const { client, emit } = setup()
    client.bodyLinks.mockImplementation(async (emailId, documentToken) => ({ documentToken, links: [{ emailId, index: 0, url }] }))
    render(<BodyDocument client={client} emailId="one" />)
    const iframe = await screen.findByTitle('邮件 HTML 正文')
    emit('token-1')
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    fireEvent.load(iframe)
    await userEvent.click(screen.getByRole('button', { name: `[1] ${url}` }))
    expect(client.requestLink).toHaveBeenCalledWith({ emailId: 'one', index: 0, documentToken: 'token-1' })
    expect(client.openLink).not.toHaveBeenCalled()
  })

  it('defaults to HTML, shows attachment type and notices, switches to plain text and invalidates prior HTML links', async () => {
    const { client, emit } = setup()
    const email: EmailDetail = {
      summary: { id: 'one', accountId: 'a', folderId: 'f', subject: 'Synthetic', from: { name: null, address: 'a@example.com' },
        receivedAt: '2026-10-20T00:00:00Z', preview: '', bodyState: 'ready', isRead: false, isStarred: false, hasAttachment: true, pendingChanges: false },
      to: [], cc: [], replyTo: [], bodyFailure: null,
      body: { text: '<script>plain text</script>', hasHtml: true, truncated: true, remoteImagesBlocked: 3 },
      attachments: [{ id: 'part', filename: 'document.pdf', mimeType: 'application/pdf', size: 2048, isInline: false }],
    }
    render(<EmailView client={client} email={email} loading={false} error={null} downloading={false} onDownload={() => undefined} />)
    await frame()
    expect(screen.getByText('已阻止 3 张远程图片')).toBeInTheDocument()
    expect(screen.getByText('正文过长，当前仅显示截断后的内容。')).toBeInTheDocument()
    expect(screen.getByText('application/pdf')).toBeInTheDocument()
    emit('token-1')
    await userEvent.click(screen.getByRole('button', { name: '取消' }))
    await userEvent.click(screen.getByRole('tab', { name: '纯文本' }))
    expect(await screen.findByText('<script>plain text</script>')).toBeInTheDocument()
    expect(screen.queryByTitle('邮件 HTML 正文')).not.toBeInTheDocument()
    emit('token-1', 'old')
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole('tab', { name: 'HTML' }))
    await waitFor(() => expect(client.prepareBody).toHaveBeenCalledTimes(2))
  })
})
