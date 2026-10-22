import { act, render, renderHook, screen, waitFor } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { QueryClient, QueryClientProvider, type InfiniteData } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { Toaster } from '@/components/ui/toast'
import { testClient } from '@/features/accounts/test-fixtures'
import type { EmailPage, EmailSummary, FlagWriteBack } from '@/generated/accounts'
import { MailxClientError } from '@/lib/mailx-client'
import { createQueryClient, mailKeys } from '@/lib/query-client'
import { useMarkOpenedAsRead, useMessageFlags } from './use-message-flags'

function email(overrides: Partial<EmailSummary> = {}): EmailSummary {
  return {
    id: 'mail-1', accountId: 'account-1', folderId: 'inbox', subject: 'Hello',
    from: { name: 'Lin', address: 'lin@example.com' }, preview: '', receivedAt: '2026-10-20T00:00:00Z',
    isRead: false, isStarred: false, hasAttachment: false, bodyState: 'ready', pendingChanges: false, ...overrides,
  }
}

const KEY = [...mailKeys.emailLists(), 'sources', { folderId: 'inbox' }]

/**
 * What a screen reader is told. base-ui hides the visible toast from assistive
 * tech (`aria-hidden`) and announces a separate copy instead: errors as
 * `role="alert"`, everything else through the polite live region. Reading that
 * copy tests what the user is actually told, not a DOM node count.
 */
const toasts = () => document.querySelectorAll('[data-slot="toast"]')

function setup(initial: EmailSummary = email(), configure?: (client: ReturnType<typeof testClient>) => void) {
  const client = testClient()
  configure?.(client)
  const queryClient: QueryClient = createQueryClient()
  queryClient.setQueryData<InfiniteData<EmailPage>>(KEY, {
    pages: [{ items: [initial], nextCursor: null, coverage: { headersComplete: true, bodiesComplete: true, syncedAfter: null, syncedBefore: null, windowCapped: false } }],
    pageParams: [null],
  })
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}<Toaster /></QueryClientProvider>
  )
  const hook = renderHook(() => useMessageFlags(client), { wrapper })
  const listed = () => queryClient.getQueryData<InfiniteData<EmailPage>>(KEY)!.pages[0].items[0]
  return { client, queryClient, hook, listed, wrapper }
}

afterEach(() => vi.useRealTimers())

describe('read and starred changes', () => {
  it('shows the new value in the list at once, before the host answers', async () => {
    let answer: (value: 'queued') => void = () => undefined
    const { client, hook, listed } = setup()
    client.setFlag.mockReturnValue(new Promise((resolve) => { answer = resolve }))
    act(() => { void hook.result.current.setFlag(listed(), 'starred', true) })
    expect(listed()).toMatchObject({ isStarred: true, pendingChanges: true })
    expect(client.setFlag).toHaveBeenCalledWith('mail-1', 'starred', true)
    await act(async () => answer('queued'))
    expect(listed().isStarred).toBe(true)
  })

  it('re-reads the folder tree after a read change, because that is where the sidebar numbers come from', async () => {
    const { client, hook, listed, queryClient } = setup()
    queryClient.setQueryData(mailKeys.tree(), { accounts: [], folders: [] })
    const spy = vi.spyOn(queryClient, 'invalidateQueries')
    await act(async () => { await hook.result.current.setFlag(listed(), 'read', true) })
    expect(client.setFlag).toHaveBeenCalledWith('mail-1', 'read', true)
    expect(spy).toHaveBeenCalledWith({ queryKey: mailKeys.tree() })
  })

  it('leaves the folder tree alone for a star, which moves no number', async () => {
    const { hook, listed, queryClient } = setup()
    const spy = vi.spyOn(queryClient, 'invalidateQueries')
    await act(async () => { await hook.result.current.setFlag(listed(), 'starred', true) })
    expect(spy).not.toHaveBeenCalledWith({ queryKey: mailKeys.tree() })
  })

  it('re-reads the tree when a read change is rolled back, so the number returns too', async () => {
    const { hook, listed, queryClient } = setup(email(), (client) => {
      client.setFlag.mockRejectedValue(new MailxClientError({ code: 'notFound', message: '内容不存在。', field: null }))
    })
    const spy = vi.spyOn(queryClient, 'invalidateQueries')
    await act(async () => { await hook.result.current.setFlag(listed(), 'read', true) })
    expect(spy).toHaveBeenCalledWith({ queryKey: mailKeys.tree() })
  })

  it('does nothing for a value the message already has', async () => {
    const { client, hook, listed } = setup(email({ isRead: true }))
    await act(async () => { await hook.result.current.setFlag(listed(), 'read', true) })
    expect(client.setFlag).not.toHaveBeenCalled()
    expect(listed().pendingChanges).toBe(false)
  })

  it('puts the old value back and says so when the change is refused', async () => {
    const { hook, listed } = setup(email(), (client) => {
      client.setFlag.mockRejectedValue(new MailxClientError({ code: 'notFound', message: '内容不存在。', field: null }))
    })
    await act(async () => { await hook.result.current.setFlag(listed(), 'read', true) })
    expect(listed()).toMatchObject({ isRead: false, pendingChanges: false })
    await waitFor(() => expect(toasts()).toHaveLength(1))
    expect(toasts()[0]).toHaveTextContent('无法更改已读状态')
    expect(toasts()[0]).toHaveTextContent('内容不存在。')
    // Errors interrupt a screen reader (role="alert"); the other kinds wait their turn.
    expect(screen.getAllByRole('alert')).toHaveLength(1)
    expect(screen.getByRole('alert')).toHaveTextContent('无法更改已读状态')
  })

  it('tells the user once that a star stays on this device, and is not left "waiting"', async () => {
    const { client, hook, listed } = setup()
    client.setFlag.mockResolvedValue('localOnly')
    await act(async () => { await hook.result.current.setFlag(listed(), 'starred', true) })
    expect(listed()).toMatchObject({ isStarred: true, pendingChanges: false })
    await waitFor(() => expect(toasts()).toHaveLength(1))
    expect(toasts()[0]).toHaveTextContent('星标仅保存在本机')
  })

  it('does not nag when a star is removed again', async () => {
    const { client, hook, listed } = setup(email({ isStarred: true }))
    client.setFlag.mockResolvedValue('localOnly')
    await act(async () => { await hook.result.current.setFlag(listed(), 'starred', false) })
    expect(listed().isStarred).toBe(false)
    expect(toasts()).toHaveLength(0)
  })

  it('refreshes the list when the host confirms a write-back and mentions kept-local stars', async () => {
    let emit: (written: FlagWriteBack) => void = () => undefined
    const { queryClient } = setup(email(), (client) => {
      client.onFlagWriteBack.mockImplementation((listener) => { emit = listener; return () => undefined })
    })
    const spy = vi.spyOn(queryClient, 'invalidateQueries')
    act(() => emit({ accountId: 'account-1', sent: 2, localOnlyStars: 0 }))
    expect(spy).toHaveBeenCalled()
    expect(toasts()).toHaveLength(0)
    act(() => emit({ accountId: 'account-1', sent: 0, localOnlyStars: 1 }))
    await waitFor(() => expect(toasts()).toHaveLength(1))
    expect(toasts()[0]).toHaveTextContent('星标仅保存在本机')
  })
})

describe('opening a message marks it read', () => {
  function Harness({ summary, shown, onFlag }: { summary: EmailSummary | null; shown: boolean; onFlag: (e: EmailSummary, f: 'read', w: boolean) => void }) {
    useMarkOpenedAsRead(summary, shown, onFlag)
    return null
  }

  it('marks an unread message once it is shown, and only once', async () => {
    const onFlag = vi.fn()
    const unread = email()
    const { rerender } = render(<Harness summary={unread} shown={false} onFlag={onFlag} />)
    await new Promise((resolve) => setTimeout(resolve, 20))
    expect(onFlag).not.toHaveBeenCalled()
    rerender(<Harness summary={unread} shown onFlag={onFlag} />)
    await waitFor(() => expect(onFlag).toHaveBeenCalledWith(unread, 'read', true))
    // The list now says read; re-rendering must not send it again.
    rerender(<Harness summary={{ ...unread, isRead: true }} shown onFlag={onFlag} />)
    await new Promise((resolve) => setTimeout(resolve, 20))
    expect(onFlag).toHaveBeenCalledTimes(1)
  })

  it('leaves read messages, messages with a change already waiting and no selection alone', async () => {
    for (const summary of [email({ isRead: true }), email({ pendingChanges: true }), null]) {
      const onFlag = vi.fn()
      render(<Harness summary={summary} shown onFlag={onFlag} />)
      await new Promise((resolve) => setTimeout(resolve, 20))
      expect(onFlag).not.toHaveBeenCalled()
    }
  })

  it('does not mark a message the user marked unread again after opening it', async () => {
    const onFlag = vi.fn()
    const { rerender } = render(<Harness summary={email({ isRead: true })} shown onFlag={onFlag} />)
    // Read, then the user toggles it back to unread while it stays open.
    rerender(<Harness summary={email({ isRead: false })} shown onFlag={onFlag} />)
    await new Promise((resolve) => setTimeout(resolve, 20))
    expect(onFlag).not.toHaveBeenCalled()
  })

  it('marks the next message when the selection moves', async () => {
    const onFlag = vi.fn()
    const { rerender } = render(<Harness summary={email({ id: 'a' })} shown onFlag={onFlag} />)
    await waitFor(() => expect(onFlag).toHaveBeenCalledTimes(1))
    rerender(<Harness summary={email({ id: 'b' })} shown onFlag={onFlag} />)
    await waitFor(() => expect(onFlag).toHaveBeenCalledTimes(2))
    expect(onFlag.mock.calls[1][0].id).toBe('b')
  })
})

describe('toast outlet', () => {
  it('shows a toast from anywhere and lets the user dismiss it', async () => {
    const { toast } = await import('@/components/ui/toast')
    render(<Toaster />)
    act(() => { toast.success('已保存', { description: '详情' }) })
    const title = await screen.findByText('已保存')
    expect(title).toBeInTheDocument()
    // The close button is hidden from assistive tech until the toast is hovered
    // or focused, which is how a reader or keyboard user reaches it.
    await userEvent.hover(title)
    await userEvent.click(await screen.findByRole('button', { name: '关闭提示' }))
    await waitFor(() => expect(screen.queryByText('已保存')).not.toBeInTheDocument())
    // The toast also announces itself politely instead of stealing focus.
    expect(screen.getByRole('region', { name: 'Notifications' })).toHaveAttribute('aria-live', 'polite')
  })
})

describe('the open message follows its own flags', () => {
  it('updates the reader\'s copy even when the list does not contain the message', async () => {
    const { hook, queryClient } = setup(email({ id: 'listed' }))
    const detail = {
      summary: email({ id: 'open-only' }), to: [], cc: [], replyTo: [], attachments: [], bodyFailure: null, body: null,
    }
    queryClient.setQueryData([...mailKeys.email('open-only'), 'identity'], detail)
    await act(async () => { await hook.result.current.setFlag(email({ id: 'open-only' }), 'starred', true) })
    const copy = queryClient.getQueryData<typeof detail>([...mailKeys.email('open-only'), 'identity'])!
    expect(copy.summary).toMatchObject({ isStarred: true, pendingChanges: true })
  })
})
