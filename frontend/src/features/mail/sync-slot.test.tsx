import { render, screen, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { exampleAccount, exampleFolderSyncState, exampleSyncState } from '@/features/accounts/test-fixtures'
import type { AccountSyncState, Folder } from '@/generated/accounts'
import { SidebarProvider } from '@/components/ui/sidebar'
import { TooltipProvider } from '@/components/ui/tooltip'
import { MailSidebar } from './mail-sidebar'

const account = exampleAccount()
const inbox: Folder = {
  id: 'inbox', accountId: account.id, parentId: null, name: 'INBOX', role: 'inbox',
  selectable: true, countsKnown: true, unreadCount: 3, totalCount: 9,
}

function Harness({ sync, onRefresh }: { sync: AccountSyncState | undefined; onRefresh: (accountId: string) => void }) {
  return (
    <TooltipProvider>
      <SidebarProvider>
        <MailSidebar
          accounts={[account]}
          folders={[inbox]}
          loading={false}
          syncStates={sync ? { [account.id]: sync } : {}}
          mailbox={{ kind: 'unified', role: 'inbox' }}
          onSelect={vi.fn()}
          onOpenSettings={vi.fn()}
          onRefresh={onRefresh}
        />
      </SidebarProvider>
    </TooltipProvider>
  )
}

function sidebar(sync: AccountSyncState | undefined, onRefresh = vi.fn()) {
  render(<Harness sync={sync} onRefresh={onRefresh} />)
  return onRefresh
}

const accountRow = () => screen.getByRole('button', { name: account.displayName }).closest('li')!
const folderRow = () => screen.getByRole('button', { name: '收件箱' }).closest('li')!

describe('account sync indicator', () => {
  it('turns a glyph on the syncing account instead of its count, and offers no refresh then', async () => {
    sidebar(exampleSyncState({
      accountId: account.id, mailboxSourceId: account.mailboxSourceId, phase: 'syncing',
      folders: [exampleFolderSyncState('inbox', { phase: 'syncing' })],
    }))
    expect(within(accountRow()).queryByRole('button', { name: /立即同步/ })).not.toBeInTheDocument()
    const status = await screen.findByRole('status', { name: '正在同步' })
    expect(status.querySelector('svg')).toHaveClass('animate-spin')
  })

  it('does not blink for a round that ends within the delay', async () => {
    const state = (phase: 'syncing' | 'idle') =>
      exampleSyncState({ accountId: account.id, mailboxSourceId: account.mailboxSourceId, phase })
    const view = render(<Harness sync={state('syncing')} onRefresh={vi.fn()} />)
    await new Promise((resolve) => setTimeout(resolve, 100))
    expect(screen.queryByRole('status', { name: '正在同步' })).not.toBeInTheDocument()
    view.rerender(<Harness sync={state('idle')} onRefresh={vi.fn()} />)
    await new Promise((resolve) => setTimeout(resolve, 500))
    expect(screen.queryByRole('status', { name: '正在同步' })).not.toBeInTheDocument()
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
    // The only refresh button is the account's own.
    expect(screen.queryAllByRole('button', { name: /立即同步/ })).toHaveLength(0)
  })

  it('shows exactly one refresh button, on the account row, when idle', () => {
    sidebar(exampleSyncState({ accountId: account.id, mailboxSourceId: account.mailboxSourceId, phase: 'idle' }))
    expect(screen.getAllByRole('button', { name: /立即同步/ })).toHaveLength(1)
    expect(within(folderRow()).queryByRole('button', { name: /同步/ })).not.toBeInTheDocument()
  })
})
