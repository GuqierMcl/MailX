import { act, render, screen, waitFor } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { QueryClientProvider } from '@tanstack/react-query'
import { describe, expect, it, vi } from 'vitest'
import { Toaster } from '@/components/ui/toast'
import { testClient } from '@/features/accounts/test-fixtures'
import type { EmailSummary, Folder, MessagesDeleted } from '@/generated/accounts'
import { t } from '@/i18n'
import { MailxClientError } from '@/lib/mailx-client'
import { createQueryClient } from '@/lib/query-client'
import { DeleteForGoodDialog } from './delete-for-good-dialog'
import { deletesForGood, useDeleteEmail } from './use-delete-email'

function email(overrides: Partial<EmailSummary> = {}): EmailSummary {
  return {
    id: 'mail-1', accountId: 'account-1', folderId: 'inbox', subject: 'Hello',
    from: { name: 'Lin', address: 'lin@example.com' }, preview: '', receivedAt: '2026-10-20T00:00:00Z',
    isRead: false, isStarred: false, hasAttachment: false, bodyState: 'ready', previewState: 'ready', pendingChanges: false, matchSnippet: null, to: { addresses: [], total: 0 }, localOnly: false, ...overrides,
  }
}

function folder(id: string, role: Folder['role'], accountId = 'account-1'): Folder {
  return { id, accountId, parentId: null, name: id, role, selectable: true, countsKnown: true, unreadCount: 0, totalCount: 0 }
}

const WITH_TRASH = [folder('inbox', 'inbox'), folder('trash', 'trash')]

function Harness({ client, folders, target, onHidden }: {
  client: ReturnType<typeof testClient>
  folders: Folder[]
  target: EmailSummary
  onHidden: (email: EmailSummary) => void
}) {
  const remove = useDeleteEmail(client, folders, onHidden)
  return (
    <>
      <button type="button" onClick={() => remove.request(target)}>delete</button>
      <DeleteForGoodDialog {...remove.confirmation} />
    </>
  )
}

function setup(options: { folders?: Folder[]; target?: EmailSummary } = {}) {
  const client = testClient()
  const onHidden = vi.fn()
  let deletions: ((deleted: MessagesDeleted) => void) | undefined
  client.onDeletions.mockImplementation((listener) => {
    deletions = listener
    return () => undefined
  })
  render(
    <QueryClientProvider client={createQueryClient()}>
      <Harness client={client} folders={options.folders ?? WITH_TRASH} target={options.target ?? email()} onHidden={onHidden} />
      <Toaster />
    </QueryClientProvider>,
  )
  return { client, onHidden, deletions: (deleted: MessagesDeleted) => act(() => deletions?.(deleted)) }
}

describe('deciding whether a delete is for good', () => {
  it('moves to Trash when the account has one and the email is elsewhere', () => {
    expect(deletesForGood(email(), WITH_TRASH)).toBe(false)
  })

  it('deletes for good inside Trash, or for an account without a usable Trash', () => {
    expect(deletesForGood(email({ folderId: 'trash' }), WITH_TRASH)).toBe(true)
    expect(deletesForGood(email(), [folder('inbox', 'inbox')])).toBe(true)
    expect(deletesForGood(email(), [folder('inbox', 'inbox'), folder('trash', 'trash', 'account-2')])).toBe(true)
    expect(deletesForGood(email(), [folder('inbox', 'inbox'), { ...folder('trash', 'trash'), selectable: false }])).toBe(true)
  })
})

describe('deleting an email', () => {
  it('hides it, then offers undo for as long as the core waits', async () => {
    const user = userEvent.setup()
    const { client, onHidden } = setup()
    await user.click(screen.getByRole('button', { name: 'delete' }))
    expect(client.deleteEmails).toHaveBeenCalledWith(['mail-1'], false)
    await waitFor(() => expect(onHidden).toHaveBeenCalledWith(expect.objectContaining({ id: 'mail-1' })))
    await user.click(await screen.findByRole('button', { name: t('mail:undo') }))
    expect(client.undoDelete).toHaveBeenCalledWith('batch-1')
  })

  it('asks before deleting for good and does nothing when the user backs out', async () => {
    const user = userEvent.setup()
    const { client } = setup({ target: email({ folderId: 'trash' }) })
    await user.click(screen.getByRole('button', { name: 'delete' }))
    expect(await screen.findByText(t('mail:deleteForGoodTitle'))).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: t('mail:cancel') }))
    expect(client.deleteEmails).not.toHaveBeenCalled()

    await user.click(screen.getByRole('button', { name: 'delete' }))
    await user.click(await screen.findByRole('button', { name: t('mail:deleteForGoodConfirm') }))
    expect(client.deleteEmails).toHaveBeenCalledWith(['mail-1'], true)
  })

  it('asks after all when the core says the delete would be for good', async () => {
    const user = userEvent.setup()
    const { client, onHidden } = setup()
    client.deleteEmails.mockRejectedValueOnce(new MailxClientError({ code: 'confirmationRequired', message: '永久删除前需要确认。', field: null }))
    await user.click(screen.getByRole('button', { name: 'delete' }))
    expect(await screen.findByText(t('mail:deleteForGoodTitle'))).toBeInTheDocument()
    expect(onHidden).not.toHaveBeenCalled()
  })

  it('says so when undo came too late', async () => {
    const user = userEvent.setup()
    const { client } = setup()
    client.undoDelete.mockRejectedValueOnce(new MailxClientError({ code: 'alreadyDeleting', message: '已开始删除，无法撤销。', field: null }))
    await user.click(screen.getByRole('button', { name: 'delete' }))
    await user.click(await screen.findByRole('button', { name: t('mail:undo') }))
    expect((await screen.findAllByText(t('mail:undoDeleteFailed')))[0]).toBeInTheDocument()
  })

  it('tells the user when the server could not delete and the emails came back', async () => {
    const { deletions } = setup()
    deletions({ accountId: 'account-1', deleted: 0, givenBack: 2 })
    expect((await screen.findAllByText(t('mail:deleteGivenBack', { count: 2 })))[0]).toBeInTheDocument()
  })
})
