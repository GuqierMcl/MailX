import { act, fireEvent, render, screen, waitFor, within } from '@/test/render'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { exampleAccount, testClient } from '@/features/accounts/test-fixtures'
import type { Draft, DraftChanged } from '@/generated/accounts'
import type { BodyLinkRequest } from '@/lib/mailx-client'
import { t } from '@/i18n'
import { DraftDetail } from './compose-details'
import { DraftBodyPreview } from './draft-body-preview'

vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, convertFileSrc: () => 'mailx-body://localhost/' }))

const account = exampleAccount()
const bodyTitle = '邮件 HTML 正文'

function exampleDraft(overrides: Partial<Draft> = {}): Draft {
  return {
    id: 'draft-one', accountId: account.id, kind: 'new', origin: 'user', sourceEmailId: null,
    to: [{ name: 'Lin', address: 'lin@example.com' }], cc: [], bcc: [], subject: 'Saved draft',
    bodyHtml: '<p><strong>Rich draft content</strong></p>', bodyText: 'Rich draft content',
    inReplyTo: null, references: null, createdAt: '2026-11-07T00:00:00Z', updatedAt: '2026-11-07T00:00:00Z',
    revision: 2, attachments: [], ...overrides,
  }
}

function setup(overrides: Partial<Draft> = {}) {
  const client = testClient([account])
  const saved = { current: exampleDraft(overrides) }
  client.getDraft.mockImplementation(async () => saved.current)
  let serial = 0
  client.prepareDraftBody.mockImplementation(async () => ({ documentToken: `draft-token-${++serial}` }))
  const draftListeners = new Set<(event: DraftChanged) => void>()
  const linkListeners = new Set<(event: BodyLinkRequest) => void>()
  client.onDrafts.mockImplementation((listener) => { draftListeners.add(listener); return () => { draftListeners.delete(listener) } })
  client.onLinkRequest.mockImplementation((listener) => { linkListeners.add(listener); return () => { linkListeners.delete(listener) } })
  const emitDraft = (event: DraftChanged) => act(() => { draftListeners.forEach((listener) => listener(event)) })
  const emitLink = (overrides: Partial<BodyLinkRequest> = {}) => act(() => {
    linkListeners.forEach((listener) => listener({
      emailId: saved.current.id, documentToken: 'draft-token-1', requestId: 'request-one', index: 0,
      url: 'https://example.com/draft-link', ...overrides,
    }))
  })
  return { client, saved, emitDraft, emitLink }
}

afterEach(() => {
  document.documentElement.classList.remove('dark')
  vi.restoreAllMocks()
})

describe('saved draft rich body preview', () => {
  it('uses an isolated draft document and lists regular attachments separately from inline images', async () => {
    const { client, saved } = setup({ attachments: [
      { id: 'pdf', fileName: 'proposal.pdf', mediaType: 'application/pdf', size: 2048, origin: 'user', inline: false, contentId: null },
      { id: 'image', fileName: 'inline-chart.png', mediaType: 'image/png', size: 4096, origin: 'user', inline: true, contentId: 'chart' },
    ] })
    const initial = await client.getSettings()
    client.getSettings.mockResolvedValue({ ...initial, settings: { ...initial.settings, security: { ...initial.settings.security, remoteImages: 'automatic' } } })
    render(<DraftDetail client={client} draftId={saved.current.id} accounts={[account]} onEdit={vi.fn()} onGone={vi.fn()} />)
    const frame = await screen.findByTitle(bodyTitle)
    fireEvent.load(frame)
    expect(frame).toHaveAttribute('src', 'mailx-body://localhost/document/draft-one?token=draft-token-1')
    expect(frame).toHaveAttribute('sandbox', '')
    expect(frame).toHaveAttribute('referrerpolicy', 'no-referrer')
    expect(frame).not.toHaveAttribute('srcdoc')
    expect(client.prepareDraftBody).toHaveBeenCalledExactlyOnceWith('draft-one', 2)
    expect(screen.getByRole('article')).not.toHaveTextContent('Rich draft content')
    const attachments = screen.getByRole('region', { name: t('mail:attachments') })
    expect(within(attachments).getAllByRole('listitem')).toHaveLength(1)
    expect(attachments).toHaveTextContent('proposal.pdf')
    expect(attachments).not.toHaveTextContent('inline-chart.png')
    expect(client.prepareBody).not.toHaveBeenCalled()
    expect(client.imageStatus).not.toHaveBeenCalled()
    expect(client.retryImages).not.toHaveBeenCalled()
    expect(client.inlineImagePreview).not.toHaveBeenCalled()
  })

  it('reloads the saved revision after a composer update and releases the previous document', async () => {
    const { client, saved, emitDraft } = setup()
    render(<DraftDetail client={client} draftId={saved.current.id} accounts={[account]} onEdit={vi.fn()} onGone={vi.fn()} />)
    await screen.findByTitle(bodyTitle)
    saved.current = { ...saved.current, revision: 3, subject: 'Updated saved draft', bodyHtml: '<h2>Updated</h2>', bodyText: 'Updated' }
    emitDraft({ draftId: saved.current.id, accountId: account.id, change: 'updated' })
    await screen.findByRole('heading', { name: 'Updated saved draft' })
    await waitFor(() => expect(screen.getByTitle(bodyTitle)).toHaveAttribute('src', 'mailx-body://localhost/document/draft-one?token=draft-token-2'))
    expect(client.prepareDraftBody).toHaveBeenLastCalledWith('draft-one', 3)
    expect(client.releaseBody).toHaveBeenCalledWith('draft-one', 'draft-token-1')
    expect(client.prepareBody).not.toHaveBeenCalled()
  })

  it('releases a late preparation after switching drafts without replacing the current preview', async () => {
    const { client, saved } = setup()
    let finish!: (value: { documentToken: string }) => void
    client.prepareDraftBody.mockReturnValueOnce(new Promise((resolve) => { finish = resolve }))
    const { rerender, unmount } = render(<DraftBodyPreview client={client} draft={saved.current} onRefresh={vi.fn()} />)
    await waitFor(() => expect(client.prepareDraftBody).toHaveBeenCalledWith('draft-one', 2))
    rerender(<DraftBodyPreview client={client} draft={exampleDraft({ id: 'draft-two', revision: 5 })} onRefresh={vi.fn()} />)
    const frame = await screen.findByTitle(bodyTitle)
    expect(frame).toHaveAttribute('src', 'mailx-body://localhost/document/draft-two?token=draft-token-1')
    await act(async () => { finish({ documentToken: 'late-draft-token' }) })
    expect(client.releaseBody).toHaveBeenCalledWith('draft-one', 'late-draft-token')
    expect(screen.getByTitle(bodyTitle)).toBe(frame)
    unmount()
    expect(client.releaseBody).toHaveBeenCalledWith('draft-two', 'draft-token-1')
  })

  it('opens current loaded links once and rejects unloaded, stale and unsafe link requests', async () => {
    const { client, saved, emitLink } = setup()
    const { rerender } = render(<DraftBodyPreview client={client} draft={saved.current} onRefresh={vi.fn()} />)
    const frame = await screen.findByTitle(bodyTitle)
    emitLink({ requestId: 'before-load' })
    expect(client.openLink).not.toHaveBeenCalled()
    fireEvent.load(frame)
    emitLink()
    emitLink()
    expect(client.openLink).toHaveBeenCalledExactlyOnceWith({ emailId: 'draft-one', documentToken: 'draft-token-1', requestId: 'request-one', index: 0 })
    rerender(<DraftBodyPreview client={client} draft={{ ...saved.current, revision: 3 }} onRefresh={vi.fn()} />)
    await waitFor(() => expect(screen.getByTitle(bodyTitle)).toHaveAttribute('src', 'mailx-body://localhost/document/draft-one?token=draft-token-2'))
    fireEvent.load(screen.getByTitle(bodyTitle))
    emitLink({ requestId: 'old-revision' })
    emitLink({ documentToken: 'draft-token-2', requestId: 'unsafe', url: 'javascript:alert(1)' })
    emitLink({ emailId: 'another-draft', documentToken: 'draft-token-2', requestId: 'wrong-draft' })
    expect(client.openLink).toHaveBeenCalledTimes(1)
    emitLink({ documentToken: 'draft-token-2', requestId: 'current-revision' })
    expect(client.openLink).toHaveBeenLastCalledWith({ emailId: 'draft-one', documentToken: 'draft-token-2', requestId: 'current-revision', index: 0 })
    expect(client.openLink).toHaveBeenCalledTimes(2)
  })

  it('keeps an empty-body hint without preparing an empty document', async () => {
    const { client, saved } = setup({ bodyHtml: '<p></p>', bodyText: '' })
    render(<DraftDetail client={client} draftId={saved.current.id} accounts={[account]} onEdit={vi.fn()} onGone={vi.fn()} />)
    expect(await screen.findByText(t('composer:detail.emptyBody'))).toBeInTheDocument()
    expect(screen.queryByTitle(bodyTitle)).not.toBeInTheDocument()
    expect(client.prepareDraftBody).not.toHaveBeenCalled()
  })

  it('renders an inline-image-only draft even when its plain text is empty', async () => {
    const { client, saved } = setup({
      bodyText: '', bodyHtml: '<p><img src="cid:chart" alt="Chart"></p>',
      attachments: [{ id: 'chart', fileName: 'chart.png', mediaType: 'image/png', size: 100, origin: 'user', inline: true, contentId: 'chart' }],
    })
    render(<DraftDetail client={client} draftId={saved.current.id} accounts={[account]} onEdit={vi.fn()} onGone={vi.fn()} />)
    expect(await screen.findByTitle(bodyTitle)).toBeInTheDocument()
    expect(screen.queryByText(t('composer:detail.emptyBody'))).not.toBeInTheDocument()
    expect(screen.queryByRole('region', { name: t('mail:attachments') })).not.toBeInTheDocument()
    expect(client.prepareDraftBody).toHaveBeenCalledWith('draft-one', 2)
  })

  it.each(['follow', 'light'] as const)('honors the %s mail theme in a dark app', async (mailTheme) => {
    const { client, saved } = setup()
    const initial = await client.getSettings()
    client.getSettings.mockResolvedValue({ ...initial, settings: { ...initial.settings, appearance: { ...initial.settings.appearance, mailTheme } } })
    document.documentElement.classList.add('dark')
    render(<DraftBodyPreview client={client} draft={saved.current} onRefresh={vi.fn()} />)
    const frame = await screen.findByTitle(bodyTitle)
    expect(new URL(frame.getAttribute('src')!).searchParams.get('scheme')).toBe(mailTheme === 'follow' ? 'dark' : null)
  })
})
