import { act, fireEvent, render, screen, waitFor } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { testClient } from '@/features/accounts/test-fixtures'
import type { BodyLinkRequest } from '@/lib/mailx-client'
import { DEFAULT_APPEARANCE } from '@/lib/appearance'
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

describe('isolated body, inline links and image policy', () => {
  it('requests the dark document only when the app is dark and mail follows it', async () => {
    const { client } = setup()
    document.documentElement.classList.add('dark')
    try {
      render(<BodyDocument client={client} emailId="one" />)
      expect(await frame()).toHaveAttribute('src', 'mailx-body://localhost/document/one?token=token-1&scheme=dark')
      vi.mocked(client.getSettings).mockResolvedValue({
        settings: { appearance: { ...DEFAULT_APPEARANCE, mailTheme: 'light' }, security: { remoteImages: 'ask' } },
        stored: true, recovered: false,
      })
    } finally {
      document.documentElement.classList.remove('dark')
    }
  })

  it('keeps the empty sandbox and opens a current inline link once without a list or dialog', async () => {
    const { client, emit } = setup()
    render(<BodyDocument client={client} emailId="one" />)
    const iframe = await frame()
    expect(iframe).toHaveAttribute('src', 'mailx-body://localhost/document/one?token=token-1')
    expect(iframe).toHaveAttribute('sandbox', '')
    expect(iframe).toHaveAttribute('referrerpolicy', 'no-referrer')
    expect(iframe).not.toHaveAttribute('srcdoc')
    emit('token-1')
    emit('token-1')
    expect(client.openLink).toHaveBeenCalledTimes(1)
    expect(client.openLink).toHaveBeenCalledWith({ emailId: 'one', index: 0, documentToken: 'token-1', requestId: 'request-1' })
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    expect(screen.queryByRole('region', { name: '正文链接' })).not.toBeInTheDocument()
    expect(client.bodyLinks).not.toHaveBeenCalled()
  })

  it('rejects events before loading, from previous selections, hidden views and reloaded documents', async () => {
    const { client, emit } = setup()
    const { rerender, unmount } = render(<BodyDocument client={client} emailId="one" />)
    await screen.findByTitle('邮件 HTML 正文')
    emit('token-1', 'before-load')
    expect(client.openLink).not.toHaveBeenCalled()
    await frame()
    rerender(<BodyDocument client={client} emailId="two" />)
    await frame()
    expect(client.releaseBody).toHaveBeenCalledWith('one', 'token-1')
    rerender(<BodyDocument client={client} emailId="one" />)
    await frame()
    expect(client.releaseBody).toHaveBeenCalledWith('two', 'token-2')
    emit('token-1', 'late')
    expect(client.openLink).not.toHaveBeenCalled()
    emit('token-3', 'current')
    expect(client.openLink).toHaveBeenCalledTimes(1)
    vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('hidden')
    fireEvent(document, new Event('visibilitychange'))
    expect(client.releaseBody).toHaveBeenCalledWith('one', 'token-3')
    emit('token-3', 'hidden')
    vi.spyOn(document, 'visibilityState', 'get').mockReturnValue('visible')
    fireEvent(document, new Event('visibilitychange'))
    await frame()
    emit('token-3', 'hidden-late')
    fireEvent.load(screen.getByTitle('邮件 HTML 正文'))
    await frame()
    emit('token-4', 'reload-late')
    expect(client.openLink).toHaveBeenCalledTimes(1)
    expect(client.releaseBody).toHaveBeenCalledWith('one', 'token-4')
    unmount()
    expect(client.releaseBody).toHaveBeenCalledWith('one', 'token-5')
  })

  it('releases a preparation that finishes after its reader has closed', async () => {
    const { client } = setup()
    let finish!: (value: { documentToken: string }) => void
    client.prepareBody.mockReturnValueOnce(new Promise((resolve) => { finish = resolve }))
    const { unmount } = render(<BodyDocument client={client} emailId="one" />)
    await waitFor(() => expect(client.prepareBody).toHaveBeenCalledTimes(1))
    unmount()
    expect(client.releaseBody).not.toHaveBeenCalled()
    await act(async () => { finish({ documentToken: 'late-token' }) })
    expect(client.releaseBody).toHaveBeenCalledExactlyOnceWith('one', 'late-token')
    expect(screen.queryByTitle('邮件 HTML 正文')).not.toBeInTheDocument()
  })

  it('asks only for remote images and scopes a one-time load to this opening', async () => {
    const { client } = setup()
    const { rerender } = render(<BodyDocument client={client} emailId="one" hasRemoteImages />)
    await frame()
    expect(client.prepareBody).toHaveBeenLastCalledWith('one', false)
    await userEvent.click(screen.getByRole('button', { name: '加载图片' }))
    await frame()
    expect(client.prepareBody).toHaveBeenLastCalledWith('one', true)
    expect(client.updateSettings).not.toHaveBeenCalled()
    expect(screen.queryByLabelText('远程图片提示')).not.toBeInTheDocument()
    rerender(<BodyDocument client={client} emailId="two" />)
    await frame()
    expect(screen.queryByLabelText('远程图片提示')).not.toBeInTheDocument()
    rerender(<BodyDocument client={client} emailId="one" hasRemoteImages />)
    await frame()
    expect(client.prepareBody).toHaveBeenLastCalledWith('one', false)
    expect(screen.getByLabelText('远程图片提示')).toBeInTheDocument()
  })

  it('checking alone and closing never fetch or save; reopening asks again', async () => {
    const { client } = setup()
    const { rerender } = render(<BodyDocument client={client} emailId="one" hasRemoteImages />)
    await frame()
    await userEvent.click(screen.getByRole('checkbox', { name: '不再提示' }))
    expect(client.updateSettings).not.toHaveBeenCalled()
    await userEvent.click(screen.getByRole('button', { name: '关闭远程图片提示' }))
    expect(screen.queryByLabelText('远程图片提示')).not.toBeInTheDocument()
    expect(client.prepareBody).toHaveBeenCalledTimes(1)
    rerender(<BodyDocument client={client} emailId="two" />)
    await frame()
    rerender(<BodyDocument client={client} emailId="one" hasRemoteImages />)
    await frame()
    expect(await screen.findByLabelText('远程图片提示')).toBeInTheDocument()
    expect(client.updateSettings).not.toHaveBeenCalled()
  })

  it('persists automatic loading only after a successful checked load', async () => {
    const { client } = setup()
    render(<BodyDocument client={client} emailId="one" hasRemoteImages />)
    await frame()
    client.updateSettings.mockRejectedValueOnce(new Error('disk error'))
    await userEvent.click(screen.getByRole('checkbox', { name: '不再提示' }))
    await userEvent.click(screen.getByRole('button', { name: '加载图片' }))
    expect(await screen.findByText('设置保存失败')).toBeInTheDocument()
    expect(client.prepareBody).toHaveBeenCalledTimes(1)
    await userEvent.click(screen.getByRole('button', { name: '加载图片' }))
    await waitFor(() => expect(client.prepareBody).toHaveBeenLastCalledWith('one', true))
    expect(client.releaseBody).toHaveBeenCalledWith('one', 'token-1')
    expect(client.updateSettings).toHaveBeenLastCalledWith({ security: { remoteImages: 'automatic' } })
    expect(screen.queryByLabelText('远程图片提示')).not.toBeInTheDocument()
  })

  it.each(['automatic', 'never'] as const)('obeys %s without a prompt', async (policy) => {
    const { client } = setup()
    const initial = await client.getSettings()
    client.getSettings.mockResolvedValue({ ...initial, settings: { ...initial.settings, security: { remoteImages: policy } } })
    render(<BodyDocument client={client} emailId="one" hasRemoteImages />)
    await frame()
    expect(client.prepareBody).toHaveBeenCalledWith('one', policy === 'automatic')
    expect(screen.queryByLabelText('远程图片提示')).not.toBeInTheDocument()
  })

  it('automatically selects HTML or plain text, preserves notices and releases the previous document', async () => {
    const { client, emit } = setup()
    const email: EmailDetail = {
      summary: { id: 'one', accountId: 'a', folderId: 'f', subject: 'Synthetic', from: { name: null, address: 'a@example.com' },
        receivedAt: '2026-10-20T00:00:00Z', preview: '', bodyState: 'ready', isRead: false, isStarred: false, hasAttachment: true, pendingChanges: false },
      to: [], cc: [], replyTo: [], bodyFailure: null,
      body: { text: '<script>plain text</script>', hasHtml: true, truncated: true, remoteImagesBlocked: 3 },
      attachments: [{ id: 'part', filename: 'document.pdf', mimeType: 'application/pdf', size: 2048, isInline: false }],
    }
    const { rerender } = render(<EmailView client={client} email={email} loading={false} error={null} downloading={false} onDownload={() => undefined} />)
    await frame()
    expect(screen.queryByRole('tablist')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '重新加载正文' })).not.toBeInTheDocument()
    expect(screen.queryByText('<script>plain text</script>')).not.toBeInTheDocument()
    expect(screen.getByLabelText('远程图片提示')).toBeInTheDocument()
    expect(screen.getByText('正文过长，当前仅显示截断后的内容。')).toBeInTheDocument()
    expect(screen.getByText('application/pdf')).toBeInTheDocument()
    emit('token-1')
    const plainEmail: EmailDetail = { ...email, summary: { ...email.summary, id: 'two' },
      body: { text: '<script>plain text</script>', hasHtml: false, truncated: false, remoteImagesBlocked: 0 } }
    rerender(<EmailView client={client} email={plainEmail} loading={false} error={null} downloading={false} onDownload={() => undefined} />)
    expect(await screen.findByText('<script>plain text</script>')).toBeInTheDocument()
    expect(screen.queryByTitle('邮件 HTML 正文')).not.toBeInTheDocument()
    expect(client.releaseBody).toHaveBeenCalledWith('one', 'token-1')
    emit('token-1', 'old')
    expect(client.openLink).toHaveBeenCalledTimes(1)
    expect(client.prepareBody).toHaveBeenCalledTimes(1)
  })
})
