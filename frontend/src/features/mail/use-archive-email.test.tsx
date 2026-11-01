import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { QueryClientProvider } from '@tanstack/react-query'
import { describe, expect, it, vi } from 'vitest'
import { Toaster } from '@/components/ui/toast'
import { testClient } from '@/features/accounts/test-fixtures'
import type { EmailSummary, Folder } from '@/generated/accounts'
import { t } from '@/i18n'
import { MailxClientError } from '@/lib/mailx-client'
import { createQueryClient } from '@/lib/query-client'
import { canArchive, useArchiveEmail } from './use-archive-email'

function email(overrides: Partial<EmailSummary> = {}): EmailSummary {
  return {
    id: 'mail-1', accountId: 'account-1', folderId: 'inbox', subject: 'Hello',
    from: { name: 'Lin', address: 'lin@example.com' }, preview: '', receivedAt: '2026-10-20T00:00:00Z',
    isRead: false, isStarred: false, hasAttachment: false, bodyState: 'ready', previewState: 'ready', pendingChanges: false, matchSnippet: null, to: { addresses: [], total: 0 }, localOnly: false, archived: false, ...overrides,
  }
}

function folder(id: string, role: Folder['role']): Folder {
  return { id, accountId: 'account-1', parentId: null, name: id, role, selectable: true, countsKnown: true, unreadCount: 0, totalCount: 0, archivedCount: 0, archivedUnreadCount: 0 }
}

function Harness({ client, target, onHidden }: {
  client: ReturnType<typeof testClient>
  target: EmailSummary
  onHidden: (email: EmailSummary) => void
}) {
  const archive = useArchiveEmail(client, onHidden)
  return <button type="button" onClick={() => void archive(target, !target.archived)}>go</button>
}

function setup(target = email()) {
  const client = testClient()
  const onHidden = vi.fn()
  render(
    <QueryClientProvider client={createQueryClient()}>
      <Harness client={client} target={target} onHidden={onHidden} />
      <Toaster />
    </QueryClientProvider>,
  )
  return { client, onHidden }
}

describe('which emails can be archived', () => {
  const folders = [folder('inbox', 'inbox'), folder('trash', 'trash')]
  it('accepts server inbox mail only', () => {
    expect(canArchive(email(), folders)).toBe(true)
    expect(canArchive(email({ folderId: 'trash' }), folders)).toBe(false)
    expect(canArchive(email({ localOnly: true }), folders)).toBe(false)
  })
})

describe('archiving an email', () => {
  it('archives at once, lets go of the email and offers an undo that unarchives', async () => {
    const user = userEvent.setup()
    const { client, onHidden } = setup()
    await user.click(screen.getByRole('button', { name: 'go' }))
    expect(client.archiveEmails).toHaveBeenCalledWith(['mail-1'])
    await waitFor(() => expect(onHidden).toHaveBeenCalledWith(expect.objectContaining({ id: 'mail-1' })))
    expect(await screen.findByText(t('mail:archived'))).toBeInTheDocument()
    await user.click(await screen.findByRole('button', { name: t('mail:undo') }))
    expect(client.unarchiveEmails).toHaveBeenCalledWith(['mail-1'])
  })

  it('moves an archived email back and undoes by archiving again', async () => {
    const user = userEvent.setup()
    const { client } = setup(email({ archived: true }))
    await user.click(screen.getByRole('button', { name: 'go' }))
    expect(client.unarchiveEmails).toHaveBeenCalledWith(['mail-1'])
    await user.click(await screen.findByRole('button', { name: t('mail:undo') }))
    expect(client.archiveEmails).toHaveBeenCalledWith(['mail-1'])
  })

  it('keeps the email and reports a failure', async () => {
    const user = userEvent.setup()
    const { client, onHidden } = setup()
    client.archiveEmails.mockRejectedValueOnce(new MailxClientError({ code: 'notFound', message: 'gone', field: null }))
    await user.click(screen.getByRole('button', { name: 'go' }))
    expect((await screen.findAllByText(t('mail:archiveFailed'))).length).toBeGreaterThan(0)
    expect(onHidden).not.toHaveBeenCalled()
  })
})
