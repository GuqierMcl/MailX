import { act, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { exampleAccount, exampleFolderSyncState, exampleSyncState } from '@/features/accounts/test-fixtures'
import type { AccountSyncState, Folder } from '@/generated/accounts'
import { TooltipProvider } from '@/components/ui/tooltip'
import { MailSidebar } from './mail-sidebar'

const account = exampleAccount()
const inbox: Folder = {
  id: 'inbox', accountId: account.id, parentId: null, name: 'INBOX', role: 'inbox',
  selectable: true, countsKnown: true, unreadCount: 3, totalCount: 9, archivedCount: 0, archivedUnreadCount: 0,
}

function Harness({ sync, onRefresh }: { sync: AccountSyncState | undefined; onRefresh: (accountId: string) => Promise<void> }) {
  return (
    <TooltipProvider>
      <MailSidebar
        accounts={[account]}
        folders={[inbox]}
        loading={false}
        syncStates={sync ? { [account.id]: sync } : {}}
        mailbox={{ kind: 'unified', role: 'inbox' }}
        onSelect={vi.fn()}
        onOpenAccounts={vi.fn()}
        onOpenSettings={vi.fn()} onCompose={vi.fn()} draftCount={0} outboxCount={0}
        onRefresh={onRefresh}
      />
    </TooltipProvider>
  )
}

function sidebar(sync: AccountSyncState | undefined, onRefresh = vi.fn().mockResolvedValue(undefined)) {
  render(<Harness sync={sync} onRefresh={onRefresh} />)
  return onRefresh
}

const accountRow = () => screen.getByRole('button', { name: account.displayName }).closest('li')!
const folderRow = () => screen.getByRole('button', { name: '收件箱' }).closest('li')!
const syncSlot = () => within(accountRow().querySelector<HTMLElement>('[data-slot="account-sync"]')!)

describe('account sync indicator', () => {
  it('keeps automatic sync silent and preserves the unread count', () => {
    sidebar(exampleSyncState({
      accountId: account.id, mailboxSourceId: account.mailboxSourceId, phase: 'syncing',
      folders: [exampleFolderSyncState('inbox', { phase: 'syncing' })],
    }))
    expect(syncSlot().getByText('3')).toBeInTheDocument()
    expect(syncSlot().queryByRole('status')).not.toBeInTheDocument()
    const refresh = syncSlot().getByRole('button', { name: `正在同步 ${account.displayName}` })
    expect(refresh).toHaveAttribute('aria-disabled', 'true')
    expect(refresh.querySelector('svg')).not.toHaveClass('animate-spin')
  })

  it('keeps automatic rounds silent across syncing and idle transitions', () => {
    const state = (phase: 'syncing' | 'idle') =>
      exampleSyncState({ accountId: account.id, mailboxSourceId: account.mailboxSourceId, phase })
    const view = render(<Harness sync={state('syncing')} onRefresh={vi.fn().mockResolvedValue(undefined)} />)
    expect(screen.queryByRole('status', { name: '正在同步' })).not.toBeInTheDocument()
    view.rerender(<Harness sync={state('idle')} onRefresh={vi.fn().mockResolvedValue(undefined)} />)
    expect(screen.queryByRole('status', { name: '正在同步' })).not.toBeInTheDocument()
  })

  it('animates a manual refresh through completion without replacing the count or animating the next automatic round', async () => {
    const onRefresh = vi.fn().mockResolvedValue(undefined)
    const state = (phase: AccountSyncState['phase'], sequence: number) => exampleSyncState({ phase, sequence })
    const view = render(<Harness sync={state('idle', 1)} onRefresh={onRefresh} />)
    await userEvent.click(syncSlot().getByRole('button', { name: `立即同步 ${account.displayName}` }))
    expect(onRefresh).toHaveBeenCalledOnce()
    expect(syncSlot().getByRole('button')).toHaveAttribute('aria-disabled', 'true')
    view.rerender(<Harness sync={state('syncing', 2)} onRefresh={onRefresh} />)
    expect((await screen.findByRole('status', { name: '正在同步' })).querySelector('svg')).toHaveClass('animate-spin')
    expect(syncSlot().getByText('3')).toBeVisible()
    await userEvent.click(syncSlot().getByRole('button'))
    expect(onRefresh).toHaveBeenCalledOnce()

    view.rerender(<Harness sync={state('idle', 3)} onRefresh={onRefresh} />)
    await waitFor(() => expect(syncSlot().queryByRole('status')).not.toBeInTheDocument(), { timeout: 1500 })
    expect(syncSlot().getByRole('button')).toHaveAttribute('aria-disabled', 'false')
    view.rerender(<Harness sync={state('syncing', 4)} onRefresh={onRefresh} />)
    expect(syncSlot().queryByRole('status')).not.toBeInTheDocument()
    expect(syncSlot().getByText('3')).toBeVisible()
  })

  it('exposes background sync and the last successful sync on hover and focus without restarting it', async () => {
    const onRefresh = sidebar(exampleSyncState({ phase: 'syncing' }))
    const refresh = syncSlot().getByRole('button', { name: `正在同步 ${account.displayName}` })
    await userEvent.hover(refresh)
    expect(await screen.findByText(/^最近同步成功：/)).toBeInTheDocument()
    await userEvent.click(refresh)
    expect(onRefresh).not.toHaveBeenCalled()
    await userEvent.unhover(refresh)
    act(() => { refresh.blur(); refresh.focus() })
    expect(await screen.findByText(`正在同步 ${account.displayName}`)).toBeInTheDocument()
    expect(refresh.querySelector('svg')).not.toHaveClass('animate-spin')
  })

  it('refreshes only that account from its own row', async () => {
    const onRefresh = sidebar(exampleSyncState({ accountId: account.id, mailboxSourceId: account.mailboxSourceId, phase: 'idle' }))
    await userEvent.click(within(accountRow()).getByRole('button', { name: `立即同步 ${account.displayName}` }))
    expect(onRefresh).toHaveBeenCalledTimes(1)
    expect(onRefresh).toHaveBeenCalledWith(account.id)
  })

  it('keeps folder rows and the unified inbox free of spinners and refresh buttons', () => {
    sidebar(exampleSyncState({
      accountId: account.id, mailboxSourceId: account.mailboxSourceId, phase: 'syncing',
      folders: [exampleFolderSyncState('inbox', { phase: 'syncing' })],
    }))
    expect(within(folderRow()).getByText('3')).toBeInTheDocument()
    expect(within(folderRow()).queryByRole('button', { name: /同步/ })).not.toBeInTheDocument()
    const unified = screen.getByRole('button', { name: '全部收件箱' }).closest('li')!
    expect(within(unified).queryByRole('button', { name: /同步/ })).not.toBeInTheDocument()
    expect(within(unified).queryByRole('status')).not.toBeInTheDocument()
    // A busy account exposes a static, unavailable action for status inspection.
    expect(screen.queryAllByRole('button', { name: /立即同步/ })).toHaveLength(0)
    expect(screen.getAllByRole('button', { name: /正在同步/ })).toHaveLength(1)
  })

  it('shows exactly one refresh button, on the account row, when idle', () => {
    sidebar(exampleSyncState({ accountId: account.id, mailboxSourceId: account.mailboxSourceId, phase: 'idle' }))
    expect(screen.getAllByRole('button', { name: /立即同步/ })).toHaveLength(1)
    expect(within(folderRow()).queryByRole('button', { name: /同步/ })).not.toBeInTheDocument()
  })
})

describe('junk folder badge', () => {
  const junk: Folder = { ...inbox, id: 'junk', name: 'Junk', role: 'junk', unreadCount: 6, totalCount: 9, archivedCount: 0, archivedUnreadCount: 0 }

  function renderWith(folders: Folder[], mailbox: Parameters<typeof MailSidebar>[0]['mailbox'] = { kind: 'unified', role: 'inbox' }) {
    render(
      <TooltipProvider>
        <MailSidebar accounts={[account]} folders={folders} loading={false} syncStates={{}} mailbox={mailbox}
          onSelect={vi.fn()} onOpenAccounts={vi.fn()} onOpenSettings={vi.fn()} onCompose={vi.fn()} draftCount={0} outboxCount={0} onRefresh={vi.fn()} />
      </TooltipProvider>,
    )
  }

  it('shows a dot instead of a number and keeps junk out of the account and inbox totals', () => {
    renderWith([inbox, junk])
    const junkRow = screen.getByRole('button', { name: '垃圾邮件' }).closest('li')!
    expect(within(junkRow).getByLabelText('有未读邮件')).toBeInTheDocument()
    expect(within(junkRow).queryByText('6')).not.toBeInTheDocument()
    // Inbox 3 unread: the account row shows 3 (not 9) and the unified inbox shows 3.
    const accountRowCount = within(screen.getByRole('button', { name: account.displayName }).closest('li')!)
    expect(accountRowCount.queryByText('9')).not.toBeInTheDocument()
    expect(accountRowCount.getAllByText('3').length).toBeGreaterThan(0)
    const unified = screen.getByRole('button', { name: '全部收件箱' }).closest('li')!
    expect(within(unified).getByText('3')).toBeInTheDocument()
  })

  it('shows nothing for junk with no unread mail and no number for an unread-only junk account', () => {
    renderWith([inbox, { ...junk, unreadCount: 0 }])
    const junkRow = screen.getByRole('button', { name: '垃圾邮件' }).closest('li')!
    expect(within(junkRow).queryByLabelText('有未读邮件')).not.toBeInTheDocument()
    expect(within(junkRow).queryByText('0')).not.toBeInTheDocument()
  })

  it('keeps a junk folder with unknown counts as "pending", not a dot or a number', () => {
    renderWith([inbox, { ...junk, countsKnown: false }])
    const junkRow = screen.getByRole('button', { name: '垃圾邮件' }).closest('li')!
    expect(within(junkRow).queryByLabelText('有未读邮件')).not.toBeInTheDocument()
  })

  it('ordinary folders still show their number', () => {
    renderWith([inbox, junk])
    const inboxRow = screen.getByRole('button', { name: '收件箱' }).closest('li')!
    expect(within(inboxRow).getByText('3')).toBeInTheDocument()
  })
})
