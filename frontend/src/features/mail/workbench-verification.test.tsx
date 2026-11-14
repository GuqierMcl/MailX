import { fireEvent, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { expect, it } from 'vitest'
import type { EmailDetail, EmailSummary, Folder } from '@/generated/accounts'
import { exampleAccount, exampleFolderSyncState, exampleSyncState, testClient } from '@/features/accounts/test-fixtures'
import { withTitleBar } from '@/test/title-bar'
import { Toaster } from '@/components/ui/toast'
import { t } from '@/i18n'
import { Workbench } from './workbench'

const account = exampleAccount()
const inbox: Folder = { id: 'inbox', accountId: account.id, parentId: null, name: 'INBOX', role: 'inbox', selectable: true, countsKnown: true, unreadCount: 1, totalCount: 1, archivedCount: 0, archivedUnreadCount: 0 }
const mail: EmailSummary = {
  id: 'otp', accountId: account.id, folderId: inbox.id, subject: 'Your verification code',
  from: { name: 'Example', address: 'login@example.test' }, to: { addresses: [], total: 0 },
  receivedAt: '2025-01-01T00:00:00Z', preview: 'Your code is 001234', isRead: false, isStarred: false,
  hasAttachment: false, bodyState: 'ready', previewState: 'ready', pendingChanges: false,
  localOnly: false, archived: false, matchSnippet: null, senderGroup: null,
}

function setup() {
  const client = testClient([account])
  let visible = true
  client.listFolders.mockResolvedValue([inbox, { ...inbox, id: 'trash', name: 'Trash', role: 'trash' }])
  client.listSyncStates.mockResolvedValue([exampleSyncState({ folders: [exampleFolderSyncState('inbox')] })])
  client.listEmails.mockImplementation(async () => ({ items: visible ? [mail] : [], nextCursor: null, coverage: { headersComplete: true, bodiesComplete: true, indexedBodies: 1, syncedAfter: null, syncedBefore: null, windowCapped: false } }))
  const detail: EmailDetail = { summary: mail, body: { text: 'Your code is 001234', hasHtml: false, remoteImagesBlocked: 0, truncated: false }, bodyFailure: null, to: [], cc: [], replyTo: [], attachments: [] }
  client.getEmail.mockResolvedValue(detail)
  client.getVerificationCode.mockImplementation(async (emailId) => ({ emailId, state: 'ready', code: '001234', recognitionId: 'recognized' }))
  client.copyVerificationCode.mockImplementation(async (emailId) => {
    visible = false
    return { emailId, accountId: account.id, cleanup: 'queued', deletion: { batchId: 'cleanup', undoUntil: new Date(Date.now() + 5000).toISOString(), permanent: false, skippedIds: [] } }
  })
  client.undoVerificationCodeDelete.mockImplementation(async () => { visible = true })
  render(withTitleBar(<><Workbench client={client} /><Toaster /></>))
  return client
}

it('copies from a card without opening or reading, then restores the hidden mail through undo', async () => {
  const client = setup()
  const list = await screen.findByRole('list', { name: t('mail:messageList') })
  await waitFor(() => expect(client.getVerificationCode).toHaveBeenCalledWith('otp', true))
  const row = within(list).getByRole('listitem')
  fireEvent.mouseEnter(row)
  const button = await within(row).findByRole('button', { name: t('mail:verificationCopy') })
  await waitFor(() => expect(row.querySelector('[data-slot="email-quick-actions"]')).toHaveAttribute('data-armed'))
  await userEvent.click(button)
  expect(client.copyVerificationCode).toHaveBeenCalledWith('otp', 'recognized')
  expect(client.getEmail).not.toHaveBeenCalled()
  expect(client.setFlag).not.toHaveBeenCalled()
  await waitFor(() => expect(screen.queryByRole('list', { name: t('mail:messageList') })).not.toBeInTheDocument())
  await userEvent.click(await screen.findByRole('button', { name: t('mail:undo') }))
  expect(client.undoVerificationCodeDelete).toHaveBeenCalledWith('cleanup')
  expect(await screen.findByRole('list', { name: t('mail:messageList') })).toBeInTheDocument()
})

it('shows the old message code in the reader and closes only that reader after cleanup', async () => {
  const client = setup()
  const list = await screen.findByRole('list', { name: t('mail:messageList') })
  await userEvent.click(within(list).getByRole('listitem'))
  const bar = await screen.findByRole('status', { name: t('mail:verificationCodeBar') })
  expect(within(bar).getByText('001234')).toBeInTheDocument()
  await userEvent.click(within(bar).getByRole('button', { name: t('mail:verificationCopy') }))
  expect(client.copyVerificationCode).toHaveBeenCalledWith('otp', 'recognized')
  expect(await screen.findByText(t('mail:noMessageSelected'))).toBeInTheDocument()
})
