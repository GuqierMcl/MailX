import { act, fireEvent, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import type { EmailDetail, EmailPage, EmailSummary, Folder } from '@/generated/accounts'
import { exampleAccount, exampleFolderSyncState, exampleSyncState, testClient } from '@/features/accounts/test-fixtures'
import { Toaster } from '@/components/ui/toast'
import { withTitleBar } from '@/test/title-bar'
import { SHORTCUTS } from './shortcuts'
import { Workbench } from './workbench'

const account = exampleAccount({ displayName: '工作邮箱' })
function folder(id: string, role: Folder['role']): Folder {
  return { id, accountId: account.id, parentId: null, name: id, role, selectable: true, countsKnown: true, unreadCount: 0, totalCount: 0, archivedCount: 0, archivedUnreadCount: 0 }
}
const inbox = folder('inbox', 'inbox')
const trash = folder('trash', 'trash')

function email(id: string, overrides: Partial<EmailSummary> = {}): EmailSummary {
  return {
    senderGroup: null,
    id, accountId: account.id, folderId: inbox.id, subject: `邮件 ${id}`,
    from: { name: '发件人', address: 'sender@example.com' }, preview: '',
    receivedAt: '2026-10-18T08:00:00.000Z', isRead: true, isStarred: false,
    hasAttachment: false, bodyState: 'ready', previewState: 'ready', pendingChanges: false, matchSnippet: null, to: { addresses: [], total: 0 }, localOnly: false, archived: false, ...overrides,
  }
}

function page(items: EmailSummary[]): EmailPage {
  return { items, nextCursor: null, coverage: { headersComplete: true, bodiesComplete: false, indexedBodies: 0, syncedAfter: null, syncedBefore: null, windowCapped: false } }
}

function detail(summary: EmailSummary): EmailDetail {
  return { summary, to: [], cc: [], replyTo: [], body: null, bodyFailure: null, attachments: [] }
}

async function setup(items: EmailSummary[], { singleKeyShortcuts = true }: { singleKeyShortcuts?: boolean } = {}) {
  const client = testClient([account])
  client.listFolders.mockResolvedValue([inbox, trash])
  client.listSyncStates.mockResolvedValue([exampleSyncState({ folders: [inbox, trash].map((row) => exampleFolderSyncState(row.id)) })])
  client.listEmails.mockResolvedValue(page(items))
  client.getEmail.mockImplementation(async (id) => detail(items.find((item) => item.id === id)!))
  if (!singleKeyShortcuts) {
    const snapshot = await client.getSettings()
    client.getSettings.mockResolvedValue({ ...snapshot, settings: { ...snapshot.settings, general: { ...snapshot.settings.general, singleKeyShortcuts: false } } })
  }
  const user = userEvent.setup()
  render(<>{withTitleBar(<Workbench client={client} />)}<Toaster /></>)
  await screen.findByRole('list', { name: '邮件列表' })
  // The setting arrives with the settings query; wait for the layout it also brings.
  await waitFor(() => expect(client.getSettings).toHaveBeenCalled())
  return { client, user }
}

const rows = () => within(screen.getByRole('list', { name: '邮件列表' })).getAllByRole('listitem')
const reader = () => within(screen.getByRole('toolbar', { name: '邮件操作' }))
const openHeading = (subject: string) => screen.findByRole('heading', { name: subject })

/** Opens `subject` by clicking its row and waits for the reader to show it. */
async function open(user: ReturnType<typeof userEvent.setup>, index: number, subject: string) {
  await user.click(rows()[index])
  expect(await openHeading(subject)).toBeInTheDocument()
  await waitFor(() => expect(reader().getByRole('button', { name: '删除' })).toBeEnabled())
  // Keys go to the page, not to the clicked row's button.
  act(() => (document.activeElement as HTMLElement | null)?.blur())
}

describe('single-key mail shortcuts', () => {
  it('j and k move the opened message like a click, marking it read the usual way', async () => {
    const { client, user } = await setup([email('a'), email('b', { isRead: false }), email('c')])
    // Nothing open: j starts at the first row.
    await user.keyboard('j')
    expect(await openHeading('邮件 a')).toBeInTheDocument()
    await user.keyboard('j')
    expect(await openHeading('邮件 b')).toBeInTheDocument()
    await waitFor(() => expect(client.setFlag).toHaveBeenCalledWith('b', 'read', true))
    await user.keyboard('{ArrowDown}')
    expect(await openHeading('邮件 c')).toBeInTheDocument()
    // At the end there is nowhere to go.
    await user.keyboard('j')
    expect(screen.getByRole('heading', { name: '邮件 c' })).toBeInTheDocument()
    await user.keyboard('k')
    expect(await openHeading('邮件 b')).toBeInTheDocument()
    await user.keyboard('{ArrowUp}')
    expect(await openHeading('邮件 a')).toBeInTheDocument()
    expect(rows()[0]).toHaveClass('mail-selected')
  })

  it('Enter and o open the first row when nothing is open', async () => {
    const { user } = await setup([email('a'), email('b')])
    await user.keyboard('o')
    expect(await openHeading('邮件 a')).toBeInTheDocument()
    await user.keyboard('{Escape}')
    await waitFor(() => expect(screen.queryByRole('heading', { name: '邮件 a' })).not.toBeInTheDocument())
    await user.keyboard('{Enter}')
    expect(await openHeading('邮件 a')).toBeInTheDocument()
  })

  it('skips sender-group rows', async () => {
    const group = email('g', { senderGroup: { address: 'news@example.com', total: 3, unread: 0 } })
    const { user } = await setup([group, email('a'), email('b')])
    await user.keyboard('j')
    expect(await openHeading('邮件 a')).toBeInTheDocument()
    await user.keyboard('k')
    expect(screen.getByRole('heading', { name: '邮件 a' })).toBeInTheDocument()
  })

  it('does nothing to messages when none is open', async () => {
    const { client, user } = await setup([email('a'), email('b')])
    await user.keyboard('esu#raf')
    await user.keyboard('{Delete}')
    expect(client.archiveEmails).not.toHaveBeenCalled()
    expect(client.deleteEmails).not.toHaveBeenCalled()
    expect(client.setFlag).not.toHaveBeenCalled()
    expect(client.createDraftFrom).not.toHaveBeenCalled()
  })

  it('s and u toggle the star and the read mark of the opened message', async () => {
    const { client, user } = await setup([email('a'), email('b')])
    await open(user, 0, '邮件 a')
    await user.keyboard('s')
    await waitFor(() => expect(client.setFlag).toHaveBeenCalledWith('a', 'starred', true))
    await user.keyboard('u')
    await waitFor(() => expect(client.setFlag).toHaveBeenCalledWith('a', 'read', false))
  })

  it('e archives the opened message and opens the next one', async () => {
    const { client, user } = await setup([email('a'), email('b'), email('c')])
    await open(user, 1, '邮件 b')
    await user.keyboard('e')
    await waitFor(() => expect(client.archiveEmails).toHaveBeenCalledWith(['b']))
    expect(await openHeading('邮件 c')).toBeInTheDocument()
  })

  it('e moves an archived message back to the inbox', async () => {
    const { client, user } = await setup([email('a', { archived: true })])
    await open(user, 0, '邮件 a')
    await user.keyboard('e')
    await waitFor(() => expect(client.unarchiveEmails).toHaveBeenCalledWith(['a']))
  })

  it('e does nothing where archive is unavailable', async () => {
    const { client, user } = await setup([email('a', { folderId: trash.id })])
    await open(user, 0, '邮件 a')
    await user.keyboard('e')
    expect(client.archiveEmails).not.toHaveBeenCalled()
  })

  it('# and Delete delete the opened message with undo, then open the next (or previous) one', async () => {
    const { client, user } = await setup([email('a'), email('b')])
    await open(user, 0, '邮件 a')
    await user.keyboard('#')
    await waitFor(() => expect(client.deleteEmails).toHaveBeenCalledWith(['a'], false))
    expect(await screen.findByText('已移到“已删除”')).toBeInTheDocument()
    expect(await openHeading('邮件 b')).toBeInTheDocument()
    await user.keyboard('{Delete}')
    await waitFor(() => expect(client.deleteEmails).toHaveBeenCalledWith(['b'], false))
    // b was last: the previous row opens.
    expect(await openHeading('邮件 a')).toBeInTheDocument()
  })

  it('Backspace in Trash asks before deleting for good', async () => {
    const { client, user } = await setup([email('a', { folderId: trash.id })])
    await open(user, 0, '邮件 a')
    await user.keyboard('{Backspace}')
    expect(await screen.findByRole('alertdialog')).toBeInTheDocument()
    expect(client.deleteEmails).not.toHaveBeenCalled()
  })

  it('# on a local sent copy asks to remove it from this device', async () => {
    const { client, user } = await setup([email('local', { localOnly: true })])
    await user.click(rows()[0])
    expect(await openHeading('邮件 local')).toBeInTheDocument()
    act(() => (document.activeElement as HTMLElement | null)?.blur())
    await user.keyboard('#')
    expect(await screen.findByRole('alertdialog')).toBeInTheDocument()
    expect(client.deleteEmails).not.toHaveBeenCalled()
    await user.keyboard('s')
    expect(client.setFlag).not.toHaveBeenCalled()
  })

  it('r, a and f reply, reply all and forward', async () => {
    const { client, user } = await setup([email('a')])
    await open(user, 0, '邮件 a')
    for (const [key, kind] of [['r', 'reply'], ['a', 'replyAll'], ['f', 'forward']] as const) {
      await user.keyboard(key)
      await waitFor(() => expect(client.createDraftFrom).toHaveBeenLastCalledWith(expect.objectContaining({ emailId: 'a', kind })))
      await waitFor(() => expect(screen.queryAllByText('无法打开写信窗口').length).toBeGreaterThan(0))
    }
    expect(client.createDraftFrom).toHaveBeenCalledTimes(3)
  })

  it('sends e, #, s and u to the picked messages while selecting', async () => {
    const { client, user } = await setup([email('a', { isRead: false }), email('b')])
    await user.click(screen.getByRole('button', { name: '选择' }))
    await user.click(rows()[0])
    await user.click(rows()[1])
    act(() => (document.activeElement as HTMLElement | null)?.blur())
    await user.keyboard('u')
    await waitFor(() => expect(client.setFlags).toHaveBeenCalledWith(expect.arrayContaining(['a', 'b']), 'read', true))
    await waitFor(() => expect(within(screen.getByRole('toolbar', { name: '所选邮件的操作' })).getByRole('button', { name: '加星标' })).toBeEnabled())
    await user.keyboard('s')
    await waitFor(() => expect(client.setFlags).toHaveBeenCalledWith(expect.arrayContaining(['a', 'b']), 'starred', true))
    await waitFor(() => expect(within(screen.getByRole('toolbar', { name: '所选邮件的操作' })).getByRole('button', { name: '归档' })).toBeEnabled())
    await user.keyboard('e')
    await waitFor(() => expect(client.archiveEmails).toHaveBeenCalledWith(['a', 'b'], true))
    expect(client.setFlag).not.toHaveBeenCalled()
  })

  it('sends # to the picked messages while selecting', async () => {
    const { client, user } = await setup([email('a'), email('b')])
    await user.click(screen.getByRole('button', { name: '选择' }))
    await user.click(rows()[1])
    act(() => (document.activeElement as HTMLElement | null)?.blur())
    await user.keyboard('#')
    await waitFor(() => expect(client.deleteEmails).toHaveBeenCalledWith(['b'], false, true))
  })

  it('? opens the overlay listing every shortcut, and ? or Esc closes it', async () => {
    const { user } = await setup([email('a')])
    await user.keyboard('?')
    const dialog = within(await screen.findByRole('dialog', { name: '键盘快捷键' }))
    for (const name of ['浏览', '处理', '回复']) expect(dialog.getByRole('heading', { name })).toBeInTheDocument()
    const listed = dialog.getAllByRole('listitem').map((item) => item.getAttribute('data-shortcut'))
    expect(listed).toEqual(SHORTCUTS.map((item) => item.id))
    // Keys do not reach the mail while it is open.
    await user.keyboard('j')
    expect(screen.queryByRole('heading', { name: '邮件 a' })).not.toBeInTheDocument()
    await user.keyboard('?')
    await waitFor(() => expect(screen.queryByRole('dialog', { name: '键盘快捷键' })).not.toBeInTheDocument())
    await user.keyboard('?')
    expect(await screen.findByRole('dialog', { name: '键盘快捷键' })).toBeInTheDocument()
    await user.keyboard('{Escape}')
    await waitFor(() => expect(screen.queryByRole('dialog', { name: '键盘快捷键' })).not.toBeInTheDocument())
  })

  it('the overlay leads to the shortcut settings', async () => {
    const { user } = await setup([email('a')])
    await user.keyboard('?')
    await user.click(await screen.findByRole('button', { name: '快捷键设置' }))
    const settings = within(await screen.findByRole('dialog', { name: '设置' }))
    expect(await settings.findByRole('switch', { name: '单键快捷键' })).toBeInTheDocument()
  })

  it('/ opens the search', async () => {
    const { user } = await setup([email('a')])
    await user.keyboard('/')
    expect(await screen.findByRole('dialog')).toBeInTheDocument()
  })
})

describe('single-key shortcut guards', () => {
  it('stay off when the setting is off', async () => {
    const { client, user } = await setup([email('a')], { singleKeyShortcuts: false })
    await waitFor(() => expect(client.getSettings).toHaveBeenCalled())
    await user.keyboard('j?')
    await new Promise((resolve) => setTimeout(resolve, 50))
    expect(screen.queryByRole('heading', { name: '邮件 a' })).not.toBeInTheDocument()
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
  })

  it('leave modifier combinations alone', async () => {
    const { client, user } = await setup([email('a')])
    await open(user, 0, '邮件 a')
    for (const modifier of ['Meta', 'Control', 'Alt']) {
      await user.keyboard(`{${modifier}>}e{/${modifier}}`)
      await user.keyboard(`{${modifier}>}r{/${modifier}}`)
    }
    expect(client.archiveEmails).not.toHaveBeenCalled()
    expect(client.createDraftFrom).not.toHaveBeenCalled()
  })

  it('leave IME composition and handled key presses alone', async () => {
    const { client, user } = await setup([email('a')])
    await open(user, 0, '邮件 a')
    fireEvent.keyDown(window, { key: 'e', isComposing: true })
    fireEvent.keyDown(window, { key: 'e', keyCode: 229 })
    const handled = new KeyboardEvent('keydown', { key: 'e', bubbles: true, cancelable: true })
    handled.preventDefault()
    window.dispatchEvent(handled)
    expect(client.archiveEmails).not.toHaveBeenCalled()
    fireEvent.keyDown(window, { key: 'e' })
    await waitFor(() => expect(client.archiveEmails).toHaveBeenCalledWith(['a']))
  })

  it('leave typing in a text field alone', async () => {
    const { client, user } = await setup([email('a')])
    await open(user, 0, '邮件 a')
    const field = document.createElement('input')
    document.body.append(field)
    field.focus()
    await user.keyboard('e#su')
    expect(field).toHaveValue('e#su')
    field.remove()
    expect(client.archiveEmails).not.toHaveBeenCalled()
    expect(client.setFlag).not.toHaveBeenCalled()
  })
})
