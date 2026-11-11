import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { fireEvent, render, screen, waitFor, within } from '@/test/render'
import type { EmailSummary } from '@/generated/accounts'
import { installBrowserDefaultGuards } from '@/lib/browser-defaults'
import { EmailList } from './email-list'
import { emailMenuActions, type EmailMenuCallbacks } from './email-menu-actions'

function email(id: string, overrides: Partial<EmailSummary> = {}): EmailSummary {
  return {
    senderGroup: null,
    id, accountId: 'work', folderId: 'inbox', subject: `主题 ${id}`,
    from: { name: 'GitHub', address: 'notifications@github.com' }, preview: '', receivedAt: '2026-10-20T00:00:00Z',
    isRead: true, isStarred: false, hasAttachment: false, bodyState: 'ready', previewState: 'ready', pendingChanges: false, matchSnippet: null, to: { addresses: [], total: 0 }, localOnly: false, archived: false, ...overrides,
  }
}

type ContactCallbacks = 'onViewContact' | 'onAggregate' | 'isAggregated' | 'viewingContact'

function everything(): Required<Omit<EmailMenuCallbacks, ContactCallbacks>> {
  return {
    onOpen: vi.fn(), onRespond: vi.fn(), onFlag: vi.fn(), onArchive: vi.fn(), onDelete: vi.fn(), onRemoveLocal: vi.fn(), canArchive: () => true,
  }
}

const kinds = (groups: ReturnType<typeof emailMenuActions>) => groups.map((group) => group.map((action) => action.kind))

describe('emailMenuActions', () => {
  it('offers every action for inbox mail, in groups', () => {
    expect(kinds(emailMenuActions(email('a'), everything()))).toEqual([
      ['open'], ['reply', 'replyAll', 'forward'], ['unread', 'star'], ['archive'], ['delete'],
    ])
  })

  it('follows the read, star and archive state', () => {
    const groups = emailMenuActions(email('a', { isRead: false, isStarred: true, archived: true }), everything())
    expect(kinds(groups)).toEqual([['open'], ['reply', 'replyAll', 'forward'], ['read', 'unstar'], ['unarchive'], ['delete']])
    expect(groups.at(-1)![0].destructive).toBe(true)
  })

  it('drops archive where the folder cannot archive locally', () => {
    const groups = emailMenuActions(email('a'), { ...everything(), canArchive: () => false })
    expect(kinds(groups)).toEqual([['open'], ['reply', 'replyAll', 'forward'], ['unread', 'star'], ['delete']])
  })

  it('offers only open, respond and removal for a local-only copy', () => {
    const groups = emailMenuActions(email('a', { localOnly: true }), everything())
    expect(kinds(groups)).toEqual([['open'], ['reply', 'replyAll', 'forward'], ['removeLocal']])
    expect(groups.at(-1)![0]).toMatchObject({ label: 'mail:removeLocalCopy', destructive: true })
  })

  it('hides the actions whose callbacks are missing and never leaves an empty group', () => {
    expect(kinds(emailMenuActions(email('a'), { onOpen: vi.fn() }))).toEqual([['open']])
    expect(kinds(emailMenuActions(email('a'), { onArchive: vi.fn() }))).toEqual([])
    expect(kinds(emailMenuActions(email('a', { localOnly: true }), { onDelete: vi.fn(), onFlag: vi.fn() }))).toEqual([])
  })

  it('calls each callback with the message', () => {
    const callbacks = everything()
    const target = email('a', { isRead: false })
    for (const action of emailMenuActions(target, callbacks).flat()) action.run()
    expect(callbacks.onOpen).toHaveBeenCalledWith(target)
    expect(callbacks.onRespond).toHaveBeenCalledWith(target, 'reply')
    expect(callbacks.onRespond).toHaveBeenCalledWith(target, 'replyAll')
    expect(callbacks.onRespond).toHaveBeenCalledWith(target, 'forward')
    expect(callbacks.onFlag).toHaveBeenCalledWith(target, 'read', true)
    expect(callbacks.onFlag).toHaveBeenCalledWith(target, 'starred', true)
    expect(callbacks.onArchive).toHaveBeenCalledWith(target, true)
    expect(callbacks.onDelete).toHaveBeenCalledWith(target)
  })
})

describe('the contact items of the menu', () => {
  const contactCallbacks = () => ({ onViewContact: vi.fn(), onAggregate: vi.fn(), isAggregated: vi.fn(() => false) })

  it('offers viewing and grouping a sender, between respond and flags', () => {
    expect(kinds(emailMenuActions(email('a'), { ...everything(), ...contactCallbacks() }))).toEqual([
      ['open'], ['reply', 'replyAll', 'forward'], ['viewContact', 'aggregate'], ['unread', 'star'], ['archive'], ['delete'],
    ])
  })

  it('offers to stop grouping once the sender has a rule', () => {
    const callbacks = { ...contactCallbacks(), isAggregated: vi.fn(() => true) }
    const target = email('a')
    const group = emailMenuActions(target, callbacks)[0]
    expect(group.map((action) => action.kind)).toEqual(['viewContact', 'unaggregate'])
    group[1].run()
    expect(callbacks.onAggregate).toHaveBeenCalledWith(target, false)
    group[0].run()
    expect(callbacks.onViewContact).toHaveBeenCalledWith(target)
  })

  it('asks to group when there is no rule', () => {
    const callbacks = contactCallbacks()
    const target = email('a')
    emailMenuActions(target, callbacks)[0][1].run()
    expect(callbacks.onAggregate).toHaveBeenCalledWith(target, true)
  })

  it('does not offer viewing the contact that is already on screen, compared without case', () => {
    const groups = emailMenuActions(email('a'), { ...contactCallbacks(), viewingContact: 'Notifications@GitHub.com' })
    expect(kinds(groups)).toEqual([['aggregate']])
  })

  it('hides the whole group for a local-only copy and for a row without a sender address', () => {
    expect(kinds(emailMenuActions(email('a', { localOnly: true }), contactCallbacks()))).toEqual([])
    expect(kinds(emailMenuActions(email('a', { from: { name: null, address: '' } }), contactCallbacks()))).toEqual([])
  })

  it('hides the items whose callbacks are missing', () => {
    expect(kinds(emailMenuActions(email('a'), { onViewContact: vi.fn() }))).toEqual([['viewContact']])
    expect(kinds(emailMenuActions(email('a'), { onAggregate: vi.fn() }))).toEqual([['aggregate']])
  })
})

describe('the message list right-click menu', () => {
  let removeGuards: () => void
  beforeEach(() => { removeGuards = installBrowserDefaultGuards() })
  afterEach(() => removeGuards())

  function renderList(emails: EmailSummary[], { stale = false } = {}) {
    const callbacks = { ...everything(), onSelect: vi.fn() }
    render(
      <EmailList
        title="全部收件箱" emails={emails} accounts={null} loading={false}
        syncStatus={{ phase: 'idle', failure: null, windowCapped: false }}
        hasMore={false} loadingMore={false} refreshing={false} stale={stale} loadMoreError={null} error={null}
        filter="all" search="" selectedId={null} previewLines="one"
        onFilterChange={vi.fn()} onSelect={callbacks.onSelect} onLoadMore={vi.fn()}
        onFlag={callbacks.onFlag} onArchive={callbacks.onArchive} onDelete={callbacks.onDelete}
        onRemoveLocal={callbacks.onRemoveLocal} canArchive={callbacks.canArchive} onRespond={callbacks.onRespond}
      />,
    )
    const list = screen.getByRole('list', { name: '邮件列表' })
    return { list, rows: within(list).getAllByRole('listitem'), callbacks }
  }

  /** Fires a right-click and reports whether the native page menu was still blocked. */
  function rightClick(target: Element, init: MouseEventInit = {}) {
    return !fireEvent.contextMenu(target, init)
  }

  it('opens on a row while the native page menu stays blocked', async () => {
    const { rows } = renderList([email('a')])
    expect(rightClick(rows[0])).toBe(true)
    const menu = await screen.findByRole('menu', { name: '邮件菜单' })
    expect(within(menu).getAllByRole('menuitem').map((item) => item.textContent)).toEqual([
      '打开', '回复', '全部回复', '转发', '标为未读', '加星标', '归档', '删除',
    ])
  })

  it('runs each item for the right-clicked message, not another row', async () => {
    const { default: userEvent } = await import('@testing-library/user-event')
    const target = email('b', { isRead: false })
    const { rows, callbacks } = renderList([email('a'), target])
    const choose = async (name: string) => {
      rightClick(rows[1])
      await userEvent.click(await screen.findByRole('menuitem', { name }))
      await waitFor(() => expect(screen.queryByRole('menu')).not.toBeInTheDocument())
    }
    await choose('打开')
    expect(callbacks.onSelect).toHaveBeenCalledWith(target)
    await choose('回复')
    expect(callbacks.onRespond).toHaveBeenCalledWith(target, 'reply')
    await choose('全部回复')
    expect(callbacks.onRespond).toHaveBeenCalledWith(target, 'replyAll')
    await choose('转发')
    expect(callbacks.onRespond).toHaveBeenCalledWith(target, 'forward')
    await choose('标为已读')
    expect(callbacks.onFlag).toHaveBeenCalledWith(target, 'read', true)
    await choose('加星标')
    expect(callbacks.onFlag).toHaveBeenCalledWith(target, 'starred', true)
    await choose('归档')
    expect(callbacks.onArchive).toHaveBeenCalledWith(target, true)
    await choose('删除')
    expect(callbacks.onDelete).toHaveBeenCalledWith(target)
    expect(callbacks.onSelect).toHaveBeenCalledTimes(1)
  })

  it('offers removal from this device for a local-only copy', async () => {
    const { default: userEvent } = await import('@testing-library/user-event')
    const target = email('a', { localOnly: true })
    const { rows, callbacks } = renderList([target])
    rightClick(rows[0])
    const menu = await screen.findByRole('menu')
    expect(within(menu).queryByRole('menuitem', { name: '删除' })).not.toBeInTheDocument()
    expect(within(menu).queryByRole('menuitem', { name: '归档' })).not.toBeInTheDocument()
    await userEvent.click(within(menu).getByRole('menuitem', { name: '从本机移除' }))
    expect(callbacks.onRemoveLocal).toHaveBeenCalledWith(target)
  })

  it('does not open or select the message by itself', async () => {
    const { rows, callbacks } = renderList([email('a')])
    rightClick(within(rows[0]).getByText('主题 a'))
    await screen.findByRole('menu')
    expect(callbacks.onSelect).not.toHaveBeenCalled()
  })

  it('does not open on the empty part of the list', () => {
    const { list } = renderList([email('a')])
    expect(rightClick(list)).toBe(true)
    expect(screen.queryByRole('menu')).not.toBeInTheDocument()
  })

  it('targets the whole row from a quick action button', async () => {
    const { default: userEvent } = await import('@testing-library/user-event')
    const { rows, callbacks } = renderList([email('a', { isRead: false }), email('b')])
    await userEvent.hover(rows[0])
    rightClick(await within(rows[0]).findByRole('button', { name: '删除' }))
    const menu = await screen.findByRole('menu')
    expect(within(menu).getByRole('menuitem', { name: '标为已读' })).toBeInTheDocument()
    expect(rows[0]).toHaveClass('bg-foreground/5')
    expect(callbacks.onDelete).not.toHaveBeenCalled()
  })

  it('keeps the row lit while the menu is open', async () => {
    const { default: userEvent } = await import('@testing-library/user-event')
    const { rows } = renderList([email('a'), email('b')])
    rightClick(rows[0])
    await screen.findByRole('menu')
    expect(rows[0]).toHaveClass('bg-foreground/5')
    expect(rows[1]).not.toHaveClass('bg-foreground/5')
    await userEvent.keyboard('{Escape}')
    await waitFor(() => expect(screen.queryByRole('menu')).not.toBeInTheDocument())
    expect(rows[0]).not.toHaveClass('bg-foreground/5')
  })

  it('closes on Escape and gives focus back to the row it was opened from', async () => {
    const { default: userEvent } = await import('@testing-library/user-event')
    const { rows } = renderList([email('a')])
    const rowButton = within(rows[0]).getAllByRole('button').find((button) => !button.hasAttribute('data-row-action'))!
    rowButton.focus()
    rightClick(rowButton)
    const menu = await screen.findByRole('menu')
    await waitFor(() => expect(menu.contains(document.activeElement)).toBe(true))
    await userEvent.keyboard('{Escape}')
    await waitFor(() => expect(screen.queryByRole('menu')).not.toBeInTheDocument())
    expect(rowButton).toHaveFocus()
  })

  it('opens at the row when the keyboard sends a point outside it', async () => {
    const { rows } = renderList([email('a')])
    vi.spyOn(rows[0], 'getBoundingClientRect').mockReturnValue(DOMRect.fromRect({ x: 10, y: 100, width: 300, height: 80 }))
    const seen: [number, number][] = []
    const record = (event: MouseEvent) => seen.push([event.clientX, event.clientY])
    window.addEventListener('contextmenu', record, true)
    try {
      expect(rightClick(rows[0], { clientX: 0, clientY: 0 })).toBe(true)
    } finally {
      window.removeEventListener('contextmenu', record, true)
    }
    expect(await screen.findByRole('menu')).toBeInTheDocument()
    // The keyboard event, then one replayed 16px in from the row's left edge, vertically centred.
    expect(seen).toEqual([[0, 0], [26, 140]])
  })

  it('shows the contact items in a normal list but not in a Sent view', async () => {
    const { default: userEvent } = await import('@testing-library/user-event')
    const contacts = { onViewContact: vi.fn(), onAggregate: vi.fn(), isAggregated: () => false }
    const props = {
      title: '收件箱', emails: [email('a')], accounts: null, loading: false,
      syncStatus: { phase: 'idle' as const, failure: null, windowCapped: false },
      hasMore: false, loadingMore: false, refreshing: false, stale: false, loadMoreError: null, error: null,
      filter: 'all' as const, search: '', selectedId: null, previewLines: 'one' as const,
      onFilterChange: vi.fn(), onSelect: vi.fn(), onLoadMore: vi.fn(), ...contacts,
    }
    const view = render(<EmailList {...props} />)
    rightClick(within(screen.getByRole('list', { name: '邮件列表' })).getAllByRole('listitem')[0])
    const menu = await screen.findByRole('menu')
    await userEvent.click(within(menu).getByRole('menuitem', { name: '查看此联系人的邮件' }))
    expect(contacts.onViewContact).toHaveBeenCalledOnce()
    expect(contacts.onViewContact.mock.calls[0][0].id).toBe('a')
    await waitFor(() => expect(screen.queryByRole('menu')).not.toBeInTheDocument())
    view.unmount()

    render(<EmailList {...props} sentView />)
    rightClick(within(screen.getByRole('list', { name: '邮件列表' })).getAllByRole('listitem')[0])
    const sentMenu = await screen.findByRole('menu')
    expect(within(sentMenu).queryByRole('menuitem', { name: /联系人/ })).not.toBeInTheDocument()
  })

  it('stays closed while the rows belong to the previous mailbox', () => {
    const { rows } = renderList([email('a')], { stale: true })
    expect(rightClick(rows[0])).toBe(true)
    expect(screen.queryByRole('menu')).not.toBeInTheDocument()
  })
})
