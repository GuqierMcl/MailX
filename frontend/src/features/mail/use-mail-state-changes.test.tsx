import { act, renderHook, screen, waitFor } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { QueryClientProvider } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { Toaster } from '@/components/ui/toast'
import { testClient } from '@/features/accounts/test-fixtures'
import type { MailStateChanged } from '@/generated/accounts'
import { MailxClientError } from '@/lib/mailx-client'
import { createQueryClient, mailKeys } from '@/lib/query-client'
import { useMailStateChanges } from './use-mail-state-changes'

const LIST = [...mailKeys.emailLists(), 'sources', { folderRole: 'inbox' }]

function setup() {
  const client = testClient()
  const queryClient = createQueryClient()
  queryClient.setQueryData(LIST, { pages: [] })
  queryClient.setQueryData(mailKeys.tree(), { accounts: [], folders: [] })
  queryClient.setQueryData(mailKeys.email('mail-1'), { summary: { id: 'mail-1' } })
  // A copy with the same Message-ID may be omitted from a bounded event.
  queryClient.setQueryData(mailKeys.email('copy-2'), { summary: { id: 'copy-2' } })
  let listener: (change: MailStateChanged) => void = () => undefined
  const stop = vi.fn()
  client.onMailStateChanged.mockImplementation((next) => { listener = next; return stop })
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}<Toaster /></QueryClientProvider>
  )
  const hook = renderHook(() => useMailStateChanges(client), { wrapper })
  const push = (overrides: Partial<MailStateChanged> = {}) => act(() => listener({
    accountIds: ['account-1'], emailIds: ['mail-1'], source: 'mcp', kind: 'archive',
    changedCount: 2, undoToken: 'receipt-1', ...overrides,
  }))
  return { client, queryClient, hook, stop, push }
}

describe('local mail changes from agents', () => {
  it('refreshes lists, folder counts and details, including unlisted copies', () => {
    const { queryClient, push } = setup()
    push({ kind: 'flags', undoToken: null })
    for (const key of [LIST, mailKeys.tree(), mailKeys.email('mail-1'), mailKeys.email('copy-2')]) {
      expect(queryClient.getQueryState(key)?.isInvalidated).toBe(true)
    }
    expect(document.querySelectorAll('[data-slot="toast"]')).toHaveLength(0)
  })

  it('shows the actual affected count and undoes with the receipt instead of reversing IDs', async () => {
    const { client, push } = setup()
    push()
    expect(await screen.findByText('MCP 已在本机归档 2 封邮件')).toBeInTheDocument()
    await userEvent.click(await screen.findByRole('button', { name: '撤销' }))
    expect(client.undoArchive).toHaveBeenCalledWith('receipt-1')
    expect(client.unarchiveEmails).not.toHaveBeenCalled()
    expect(client.archiveEmails).not.toHaveBeenCalled()
  })

  it('explains an AI unarchive without a restoration button when no receipt is available', async () => {
    const { push } = setup()
    push({ source: 'ai', kind: 'unarchive', changedCount: 1, undoToken: null })
    expect(await screen.findByText('AI 已在本机将 1 封邮件移回收件箱')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '撤销' })).not.toBeInTheDocument()
  })

  it('does not duplicate UI notices or announce unchanged/undo operations', () => {
    const { push } = setup()
    push({ source: 'user' })
    push({ changedCount: 0 })
    push({ kind: 'undoArchive' })
    expect(document.querySelectorAll('[data-slot="toast"]')).toHaveLength(0)
  })

  it('reports a failed restore without falling back to an unsafe inverse mutation', async () => {
    const { client, push } = setup()
    client.undoArchive.mockRejectedValueOnce(new MailxClientError({ code: 'conflict', message: '', field: null }))
    push()
    await userEvent.click(await screen.findByRole('button', { name: '撤销' }))
    await waitFor(() => expect(screen.getAllByText('无法撤销此次归档操作').length).toBeGreaterThan(0))
    expect(client.unarchiveEmails).not.toHaveBeenCalled()
    expect(client.archiveEmails).not.toHaveBeenCalled()
  })

  it('unsubscribes when the workbench goes away', () => {
    const { hook, stop } = setup()
    hook.unmount()
    expect(stop).toHaveBeenCalledOnce()
  })
})
