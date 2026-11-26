import { act, fireEvent, render, screen, waitFor } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { testClient } from '@/features/accounts/test-fixtures'
import type { BodyLinkRequest } from '@/lib/mailx-client'
import { DEFAULT_APPEARANCE } from '@/lib/appearance'
import { BodyDocument } from './body-document'
import { ImageDetailsSlot, summarizeImageFailures } from './image-status'
import { EmailView } from './email-view'
import type { EmailDetail, RemoteImageStatus } from '@/generated/accounts'
import { DEFAULT_BADGE, DEFAULT_NOTIFICATIONS } from '@/features/notifications/defaults'
import { DEFAULT_NOW } from '@/features/now/modules/defaults'
import { DEFAULT_LAYOUT } from '@/features/shell/layout-defaults'

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

describe('reader image failure severity', () => {
  const group = (index: number, failed = true): RemoteImageStatus => ({
    index, group: index, role: 'content', loading: false, error: failed ? 'connect' : null,
  })

  it.each([
    [2, 2, false], [3, 6, true], [3, 7, false],
  ] as const)('summarizes %i failed of %i completed image groups', (failed, total, severe) => {
    const statuses = Array.from({ length: total }, (_, index) => group(index, index < failed))
    expect(summarizeImageFailures(statuses)).toMatchObject({ failed, severe })
  })

  it('counts duplicate addresses once, excludes decorative images, and waits for in-flight groups', () => {
    const statuses: RemoteImageStatus[] = [
      group(0), { ...group(1), group: 0 }, group(2), { ...group(3), role: 'decorative' },
      { ...group(4), loading: true, error: null },
    ]
    expect(summarizeImageFailures(statuses)).toMatchObject({ failed: 3, severe: false })
    statuses.push({ ...group(5), group: 0, role: 'important' })
    expect(summarizeImageFailures(statuses)).toMatchObject({ failed: 3, severe: true })
    statuses.push({ ...group(6, false), group: 0 })
    expect(summarizeImageFailures(statuses)).toMatchObject({ failed: 2, severe: false })
  })
})

describe('isolated body, inline links and image policy', () => {
  it('lets the reader retry a failed document and replaces the notice with its body', async () => {
    const { client } = setup()
    client.prepareBody.mockRejectedValueOnce(new Error('prepare failed'))
    render(<BodyDocument client={client} emailId="one" />)
    expect(await screen.findByRole('alert')).toHaveTextContent('暂时无法显示邮件正文。')
    await userEvent.click(screen.getByRole('button', { name: '重试' }))
    await frame()
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
    expect(client.prepareBody).toHaveBeenCalledTimes(2)
  })

  it('keeps the document readable after a link fails and clears the notice when a link opens', async () => {
    const { client, emit } = setup()
    let resolveFirst!: () => void
    client.openLink.mockReturnValueOnce(new Promise<void>((resolve) => { resolveFirst = resolve }))
    client.openLink.mockRejectedValueOnce(new Error('open failed'))
    render(<BodyDocument client={client} emailId="one" />)
    const body = await frame()
    emit('token-1', 'slow-request')
    emit('token-1')
    expect(await screen.findByRole('alert')).toHaveTextContent('无法打开此链接。')
    await act(async () => resolveFirst())
    expect(screen.getByRole('alert')).toHaveTextContent('无法打开此链接。')
    expect(screen.getByTitle('邮件 HTML 正文')).toBe(body)
    emit('token-1', 'next-request')
    await waitFor(() => expect(screen.queryByRole('alert')).not.toBeInTheDocument())
    expect(client.prepareBody).toHaveBeenCalledTimes(1)
  })

  it('keeps a decorative failure quiet and allows inspection and retry with the existing grant', async () => {
    const { client } = setup()
    const pixel = { index: 0, group: 0, role: 'decorative', loading: false } as const
    client.imageStatus.mockResolvedValue([{ ...pixel, error: 'proxyPolicyRequired' }])
    render(<ImageDetailsSlot.Provider value={document.body}><BodyDocument client={client} emailId="one" hasRemoteImages /></ImageDetailsSlot.Provider>)
    await frame()
    expect(client.imageStatus).not.toHaveBeenCalled()
    await userEvent.click(screen.getByRole('button', { name: '加载图片' }))
    await frame()
    await userEvent.click(await screen.findByRole('button', { name: '图片加载状态' }))
    const details = await screen.findByRole('dialog', { name: '图片加载状态' })
    expect(details).toHaveTextContent('1 张图片未能加载')
    expect(details).toHaveTextContent('账号代理设置')
    expect(screen.queryByText('部分邮件图片未能显示。')).not.toBeInTheDocument()
    client.imageStatus.mockResolvedValue([{ ...pixel, error: null }])
    await userEvent.click(screen.getByRole('button', { name: '重试图片' }))
    await waitFor(() => expect(screen.getByTitle('邮件 HTML 正文')).toHaveAttribute('src', 'mailx-body://localhost/document/one?token=retry-document'))
    await frame()
    expect(client.prepareBody).toHaveBeenLastCalledWith('one', true)
    expect(client.retryImages).toHaveBeenCalledExactlyOnceWith('one', 'token-2')
    expect(screen.getByTitle('邮件 HTML 正文')).toHaveAttribute('src', 'mailx-body://localhost/document/one?token=retry-document')
    expect(client.updateSettings).not.toHaveBeenCalled()
    expect(screen.queryByLabelText('远程图片提示')).not.toBeInTheDocument()
    await waitFor(() => expect(screen.queryByLabelText('图片加载状态')).not.toBeInTheDocument())
  })

  it('ignores diagnostics completing after a different message was selected', async () => {
    const { client } = setup()
    const initial = await client.getSettings()
    client.getSettings.mockResolvedValue({ ...initial, settings: { ...initial.settings, security: { remoteImages: 'automatic', senderAvatars: false } } })
    let finish!: (value: RemoteImageStatus[]) => void
    client.imageStatus.mockReturnValueOnce(new Promise((resolve) => { finish = resolve }))
    const { rerender } = render(<BodyDocument client={client} emailId="one" hasRemoteImages />)
    await waitFor(() => expect(client.imageStatus).toHaveBeenCalled())
    rerender(<BodyDocument client={client} emailId="two" />)
    await frame()
    await act(async () => { finish([{ index: 0, group: 0, role: 'important', loading: false, error: 'tls' }]) })
    expect(screen.queryByText('部分邮件图片未能显示。')).not.toBeInTheDocument()
    expect(screen.queryByLabelText('图片加载状态')).not.toBeInTheDocument()
  })

  it('announces confirmed failures at the observation deadline without retrying pending resources', async () => {
    const { client } = setup()
    client.imageStatus.mockResolvedValue([
      { index: 0, group: 0, role: 'important', loading: false, error: 'tls' },
      { index: 1, group: 1, role: 'content', loading: true, error: null },
    ])
    render(<BodyDocument client={client} emailId="one" hasRemoteImages />)
    await frame()
    await userEvent.click(screen.getByRole('button', { name: '加载图片' }))
    await screen.findByTitle('邮件 HTML 正文')
    vi.useFakeTimers()
    try {
      fireEvent.load(screen.getByTitle('邮件 HTML 正文'))
      await act(async () => { await vi.advanceTimersByTimeAsync(30_000) })
      expect(screen.queryByText('部分邮件图片未能显示。')).not.toBeInTheDocument()
      await act(async () => { await vi.advanceTimersByTimeAsync(5_000) })
      expect(screen.getByRole('status')).toHaveTextContent('部分邮件图片未能显示。')
      expect(screen.getByRole('button', { name: '重试图片' })).toBeDisabled()
      fireEvent.click(screen.getByRole('button', { name: '关闭图片加载提示' }))
      expect(screen.queryByText('部分邮件图片未能显示。')).not.toBeInTheDocument()
    } finally {
      vi.useRealTimers()
    }
  })

  it('inspects and retries only the visible document, and quiets a banner after partial recovery', async () => {
    const { client } = setup()
    const initial = await client.getSettings()
    client.getSettings.mockResolvedValue({ ...initial, settings: { ...initial.settings, security: { remoteImages: 'automatic', senderAvatars: false } } })
    client.prepareBody.mockImplementation(async (_id, _allowImages, language) => ({ documentToken: language ? 'translated' : 'original' }))
    const small: RemoteImageStatus = { index: 1, group: 1, role: 'decorative', error: 'unsupportedType', loading: false }
    const cover: RemoteImageStatus = { index: 0, group: 0, role: 'important', error: 'tls', loading: false }
    client.imageStatus.mockImplementation(async (_id, token) => token === 'original' ? [small] :
      [{ ...cover, error: token === 'retry-document' ? null : cover.error }, small])
    const documentView = (showing: boolean) => <ImageDetailsSlot.Provider value={document.body}>
      <BodyDocument client={client} emailId="one" hasRemoteImages translated={{ target: 'en', version: 1, showing, onFailed: vi.fn() }} />
    </ImageDetailsSlot.Provider>
    const view = render(documentView(false))
    await frame()
    await screen.findByRole('button', { name: '图片加载状态' })
    expect(screen.queryByText('部分邮件图片未能显示。')).not.toBeInTheDocument()
    view.rerender(documentView(true))
    fireEvent.load(await screen.findByTitle('邮件 HTML 正文（译文）'))
    await screen.findByText('部分邮件图片未能显示。')
    expect(screen.getAllByRole('button', { name: '图片加载状态' })).toHaveLength(1)
    await userEvent.click(screen.getByRole('button', { name: '重试图片' }))
    expect(client.retryImages).toHaveBeenCalledExactlyOnceWith('one', 'translated')
    await waitFor(() => expect(screen.getByTitle('邮件 HTML 正文（译文）')).toHaveAttribute('src', 'mailx-body://localhost/document/one?token=retry-document'))
    fireEvent.load(screen.getByTitle('邮件 HTML 正文（译文）'))
    await userEvent.click(await screen.findByRole('button', { name: '图片加载状态' }))
    expect(await screen.findByRole('dialog', { name: '图片加载状态' })).toHaveTextContent('1 张图片未能加载')
    expect(screen.queryByText('部分邮件图片未能显示。')).not.toBeInTheDocument()
    expect(screen.getByTitle('邮件 HTML 正文')).toHaveAttribute('src', 'mailx-body://localhost/document/one?token=original')
  })

  it('releases a retry token that arrives after the reader closed', async () => {
    const { client } = setup()
    client.imageStatus.mockResolvedValue([{ index: 0, group: 0, role: 'important', loading: false, error: 'connect' }])
    let finish!: (value: { documentToken: string }) => void
    client.retryImages.mockReturnValueOnce(new Promise((resolve) => { finish = resolve }))
    const { unmount } = render(<BodyDocument client={client} emailId="one" hasRemoteImages />)
    await frame()
    await userEvent.click(screen.getByRole('button', { name: '加载图片' }))
    await frame()
    await userEvent.click(await screen.findByRole('button', { name: '重试图片' }))
    unmount()
    await act(async () => { finish({ documentToken: 'late-retry' }) })
    expect(client.releaseBody).toHaveBeenCalledWith('one', 'late-retry')
    expect(screen.queryByTitle('邮件 HTML 正文')).not.toBeInTheDocument()
  })

  it('requests the dark document only when the app is dark and mail follows it', async () => {
    const { client } = setup()
    document.documentElement.classList.add('dark')
    try {
      render(<BodyDocument client={client} emailId="one" />)
      expect(await frame()).toHaveAttribute('src', 'mailx-body://localhost/document/one?token=token-1&scheme=dark')
      vi.mocked(client.getSettings).mockResolvedValue({
        settings: { general: { language: 'zh-CN', autoUpdate: true, closeBehavior: 'ask', showMenuBarIcon: false, silentStart: false, singleKeyShortcuts: true, startupView: 'inbox' }, mail: { trashVerificationMailAfterCopy: false }, appearance: { ...DEFAULT_APPEARANCE, mailTheme: 'light' }, security: { remoteImages: 'ask', senderAvatars: false }, layout: DEFAULT_LAYOUT, notifications: DEFAULT_NOTIFICATIONS, badge: DEFAULT_BADGE, now: DEFAULT_NOW, compose: { defaultAccountId: '', undoSendSeconds: 0, openMode: 'window' }, mcp: { enabled: false }, assistant: { enabled: true, model: null, consented: null, excludedAccounts: [], organize: 'direct' as const, aiDrafts: 'direct' as const, send: 'confirm' as const, instructions: '', saveHistory: true, readHtml: false, summary: { auto: false, senders: [], style: 'brief' as const, language: { mode: 'interface' as const }, collapsed: false } }, translation: { provider: null, microsoftRegion: '', route: { kind: 'direct' }, targetLanguage: 'auto', suggest: true, neverSuggest: [], alwaysTranslate: [], includeQuotes: false, consentedProvider: null, consentedAi: null } },
        effectiveLocale: 'zh-CN', revision: 0, stored: true, recovered: false,
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
    expect(client.updateSettings).toHaveBeenLastCalledWith({ security: { remoteImages: 'automatic', senderAvatars: false } })
    expect(screen.queryByLabelText('远程图片提示')).not.toBeInTheDocument()
  })

  it.each(['automatic', 'never'] as const)('obeys %s without a prompt', async (policy) => {
    const { client } = setup()
    const initial = await client.getSettings()
    client.getSettings.mockResolvedValue({ ...initial, settings: { ...initial.settings, security: { remoteImages: policy, senderAvatars: false } } })
    render(<BodyDocument client={client} emailId="one" hasRemoteImages />)
    await frame()
    expect(client.prepareBody).toHaveBeenCalledWith('one', policy === 'automatic')
    expect(screen.queryByLabelText('远程图片提示')).not.toBeInTheDocument()
  })

  it('automatically selects HTML or plain text, preserves notices and releases the previous document', async () => {
    const { client, emit } = setup()
    const email: EmailDetail = {
      summary: { id: 'one', accountId: 'a', folderId: 'f', subject: 'Synthetic', from: { name: null, address: 'a@example.com' },
        receivedAt: '2026-10-20T00:00:00Z', preview: '', bodyState: 'ready', previewState: 'ready', isRead: false, isStarred: false, hasAttachment: true, pendingChanges: false, matchSnippet: null, senderGroup: null, to: { addresses: [], total: 0 }, localOnly: false, archived: false },
      to: [], cc: [], replyTo: [], bodyFailure: null,
      body: { text: '<script>plain text</script>', hasHtml: true, truncated: true, remoteImagesBlocked: 3 },
      attachments: [{ id: 'part', filename: 'document.pdf', mimeType: 'application/pdf', size: 2048, isInline: false }],
    }
    const { rerender } = render(<EmailView client={client} email={email} loading={false} error={null} downloading={false} onDownload={() => undefined} onFlag={() => undefined} onClose={() => undefined} onRespond={() => undefined} />)
    await frame()
    expect(screen.queryByRole('tablist')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '重新加载正文' })).not.toBeInTheDocument()
    expect(screen.queryByText('<script>plain text</script>')).not.toBeInTheDocument()
    expect(screen.getByLabelText('远程图片提示')).toBeInTheDocument()
    expect(screen.getByText('正文过长，当前仅显示截断后的内容。')).toBeInTheDocument()
    // Name and size are shown; the sender-claimed type is not, since it can lie.
    expect(screen.getByText('document.pdf')).toBeInTheDocument()
    expect(screen.getByText('2 KB')).toBeInTheDocument()
    expect(screen.queryByText('application/pdf')).not.toBeInTheDocument()
    emit('token-1')
    const plainEmail: EmailDetail = { ...email, summary: { ...email.summary, id: 'two' },
      body: { text: '<script>plain text</script>', hasHtml: false, truncated: false, remoteImagesBlocked: 0 } }
    rerender(<EmailView client={client} email={plainEmail} loading={false} error={null} downloading={false} onDownload={() => undefined} onFlag={() => undefined} onClose={() => undefined} onRespond={() => undefined} />)
    expect(await screen.findByText('<script>plain text</script>')).toBeInTheDocument()
    expect(screen.queryByTitle('邮件 HTML 正文')).not.toBeInTheDocument()
    expect(client.releaseBody).toHaveBeenCalledWith('one', 'token-1')
    emit('token-1', 'old')
    expect(client.openLink).toHaveBeenCalledTimes(1)
    expect(client.prepareBody).toHaveBeenCalledTimes(1)
  })
})
