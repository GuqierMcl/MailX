import { act, fireEvent, render, screen, waitFor, within } from '@/test/render'
import { describe, expect, it, vi } from 'vitest'
import type { EmailDetail, EmailSummary, VerificationCodeStatus } from '@/generated/accounts'
import { testClient } from '@/features/accounts/test-fixtures'
import { TooltipProvider } from '@/components/ui/tooltip'
import { t } from '@/i18n'
import { EmailCard } from './email-card'
import { EmailView } from './email-view'
import { EmailList } from './email-list'
import { testViewport } from '@/test/viewport'
import type { VerificationCodes } from './use-verification-codes'

vi.mock('./body-document', () => ({
  BodyDocument: ({ banner }: { banner?: React.ReactNode }) => <div data-testid="html-reader">{banner}<iframe title="mail body" /></div>,
  BodySkeleton: () => <div>loading body</div>,
}))

function email(overrides: Partial<EmailSummary> = {}): EmailSummary {
  return {
    id: 'otp', accountId: 'a', folderId: 'inbox', subject: 'Your verification code',
    from: { name: 'Example', address: 'login@example.test' }, to: { addresses: [], total: 0 },
    receivedAt: '2025-01-01T00:00:00Z', preview: '', isRead: false, isStarred: true,
    hasAttachment: false, bodyState: 'ready', previewState: 'ready', pendingChanges: false,
    localOnly: false, archived: false, matchSnippet: null, senderGroup: null, ...overrides,
  }
}

const ready = (emailId = 'otp'): VerificationCodeStatus => ({ emailId, state: 'ready', code: '001234', recognitionId: `recognition-${emailId}` })
function controls(status: VerificationCodeStatus | undefined = ready()): VerificationCodes {
  return { status: () => status, requestVisible: vi.fn(), copy: vi.fn().mockResolvedValue(undefined), busy: () => false, copied: () => false }
}

function cardProps(verification = controls()) {
  return { email: email(), sentView: false, account: null, selected: false, previewLines: 'one' as const,
    index: 0, count: 1, style: {}, ref: null, verification,
    onFlag: vi.fn(), onArchive: vi.fn(), canArchive: () => true, onDelete: vi.fn(), onSelect: vi.fn() }
}

describe('verification code quick actions', () => {
  it('requests only visible individual rows, excluding offscreen pinned rows', async () => {
    testViewport.height = 600
    const verification = controls()
    const emails = Array.from({ length: 500 }, (_, index) => email({ id: `mail-${index}`, subject: `subject ${index}` }))
    emails[1] = email({ id: 'group', senderGroup: { address: 'login@example.test', total: 2, unread: 1 } })
    emails[2] = email({ id: 'local', localOnly: true })
    render(<EmailList title="Inbox" emails={emails} accounts={null} loading={false}
      syncStatus={{ phase: 'idle', failure: null, windowCapped: false }} hasMore={false} loadingMore={false}
      refreshing={false} stale={false} loadMoreError={null} error={null} filter="all" search=""
      selectedId="mail-0" previewLines="one" onFilterChange={vi.fn()} onSelect={vi.fn()} onLoadMore={vi.fn()}
      verification={verification} />)
    await waitFor(() => expect(verification.requestVisible).toHaveBeenCalled())
    const last = () => vi.mocked(verification.requestVisible).mock.calls.at(-1)![0]
    expect(last().length).toBeGreaterThan(0)
    expect(last().length).toBeLessThan(12)
    expect(last().some(({ emailId }) => emailId === 'group' || emailId === 'local')).toBe(false)
    const viewport = document.querySelector<HTMLElement>('[data-slot="scroll-area-viewport"]')!
    await act(async () => { viewport.scrollTop = 30000; viewport.dispatchEvent(new Event('scroll')) })
    await waitFor(() => expect(last().some(({ emailId }) => emailId === 'mail-0')).toBe(false))
    expect(screen.getByText('subject 0')).toBeInTheDocument()
  })
  it('replaces three controls, retains delete and star marker, and does not open or mark read', async () => {
    const props = cardProps()
    render(<TooltipProvider><EmailCard {...props} /></TooltipProvider>)
    const row = screen.getByRole('listitem')
    fireEvent.mouseEnter(row)
    const copy = await screen.findByRole('button', { name: t('mail:verificationCopy') })
    await waitFor(() => expect(row.querySelector('[data-slot="email-quick-actions"]')).toHaveAttribute('data-armed'))
    expect(within(row).queryByRole('button', { name: t('mail:unstar') })).not.toBeInTheDocument()
    expect(within(row).queryByRole('button', { name: t('mail:markRead') })).not.toBeInTheDocument()
    expect(within(row).queryByRole('button', { name: t('mail:archive') })).not.toBeInTheDocument()
    expect(within(row).getByRole('button', { name: t('mail:delete') })).toBeInTheDocument()
    expect(within(row).getByLabelText(t('mail:starred'))).toBeInTheDocument()
    fireEvent.click(copy)
    expect(props.verification.copy).toHaveBeenCalledWith('otp', 'recognition-otp')
    expect(props.onFlag).not.toHaveBeenCalled()
    expect(props.onSelect).not.toHaveBeenCalled()
  })

  it('adopts a late recognition only after the active toolbar closes', async () => {
    const props = cardProps(controls(undefined))
    // Passing undefined uses the factory default; this status is explicitly unknown.
    props.verification.status = () => undefined
    const { rerender } = render(<TooltipProvider><EmailCard {...props} /></TooltipProvider>)
    const row = screen.getByRole('listitem')
    fireEvent.mouseEnter(row)
    expect(await screen.findByRole('button', { name: t('mail:unstar') })).toBeInTheDocument()
    props.verification.status = () => ready()
    rerender(<TooltipProvider><EmailCard {...props} /></TooltipProvider>)
    expect(screen.queryByRole('button', { name: t('mail:verificationCopy') })).not.toBeInTheDocument()
    fireEvent.mouseLeave(row)
    await waitFor(() => expect(row.querySelector('[data-slot="email-quick-actions"]')).not.toBeInTheDocument())
    fireEvent.mouseEnter(row)
    expect(await screen.findByRole('button', { name: t('mail:verificationCopy') })).toBeInTheDocument()
  })

  it.each(['group', 'selection', 'local'] as const)('never exposes copy on a %s row', async (kind) => {
    const props = cardProps()
    const item = kind === 'group' ? email({ senderGroup: { address: 'login@example.test', total: 3, unread: 2 } })
      : kind === 'local' ? email({ localOnly: true }) : email()
    render(<TooltipProvider><EmailCard {...props} email={item} selectionMode={kind === 'selection'} /></TooltipProvider>)
    fireEvent.mouseEnter(screen.getByRole('listitem'))
    expect(screen.queryByRole('button', { name: t('mail:verificationCopy') })).not.toBeInTheDocument()
  })
})

describe('verification code reader notice', () => {
  it.each([false, true])('uses the expanded message identity for %s HTML body', async (hasHtml) => {
    const verification = controls()
    verification.status = (id) => ready(id)
    const detail = (id: string): EmailDetail => ({ summary: email({ id, isRead: true, archived: true }), body: { text: 'Body', hasHtml, truncated: false, remoteImagesBlocked: 0 }, bodyFailure: null, to: [], cc: [], replyTo: [], attachments: [] })
    const props = { client: testClient(), email: detail('first'), verification, loading: false, error: null, downloading: false,
      onDownload: vi.fn(), onFlag: vi.fn(), onClose: vi.fn(), onRespond: vi.fn() }
    const { rerender } = render(<TooltipProvider><EmailView {...props} /></TooltipProvider>)
    const notice = screen.getByRole('status', { name: t('mail:verificationCodeBar') })
    expect(within(notice).getByText('001234')).toHaveClass('font-mono')
    expect(notice.closest('iframe')).toBeNull()
    fireEvent.click(within(notice).getByRole('button', { name: t('mail:verificationCopy') }))
    expect(verification.copy).toHaveBeenLastCalledWith('first', 'recognition-first')
    rerender(<TooltipProvider><EmailView {...props} email={detail('second')} /></TooltipProvider>)
    fireEvent.click(screen.getByRole('button', { name: t('mail:verificationCopy') }))
    expect(verification.copy).toHaveBeenLastCalledWith('second', 'recognition-second')
  })
})
