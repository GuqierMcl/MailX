import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { testClient } from '@/features/accounts/test-fixtures'
import type { DetectedActionItem, EmailSummary } from '@/generated/accounts'
import { ActionItemDialog } from './action-item-dialog'
import { ActionItemsBar, ActionItemsPromptBanner } from './action-items-bar'
import type { EmailActionItemsState } from './use-email-action-items'

afterEach(() => {
  cleanup()
})

const SAMPLE_SUMMARY: EmailSummary = {
  id: 'email-1',
  accountId: 'acc-1',
  folderId: 'folder-1',
  from: { name: 'Alice', address: 'alice@example.com' },
  to: { addresses: [], total: 0 },
  subject: 'Q4 Review',
  preview: 'Let us meet tomorrow',
  previewState: 'ready',
  receivedAt: '2026-11-28T02:00:00Z',
  isRead: true,
  isStarred: false,
  archived: false,
  hasAttachment: false,
  bodyState: 'ready',
  pendingChanges: false,
  localOnly: false,
  matchSnippet: null,
  senderGroup: null,
}

function makeItem(overrides: Partial<DetectedActionItem> = {}): DetectedActionItem {
  return {
    id: 'ai:event:2026-12-03:abc',
    kind: 'event',
    source: 'ai',
    status: 'pending',
    title: 'Q4 Product Review',
    startAt: '2026-12-03T14:30:00+08:00',
    endAt: '2026-12-03T15:30:00+08:00',
    allDay: false,
    rrule: null,
    cancelled: false,
    location: 'Room 302',
    url: 'https://meet.example.com/q4',
    notes: 'Bring Q4 slides',
    matchedText: 'Dec 3 at 2:30 PM',
    ...overrides,
  }
}

function makeState(overrides: Partial<EmailActionItemsState> = {}): EmailActionItemsState {
  return {
    aiAvailable: true,
    phase: 'done',
    error: null,
    service: 'OpenAI',
    icsItems: [],
    aiItems: [],
    l1Items: [],
    visibleItems: [],
    showPromptBanner: false,
    showEmptyNotice: false,
    autoExtract: false,
    editingItem: null,
    setL1Items: vi.fn(),
    openItem: vi.fn(),
    closeDialog: vi.fn(),
    extract: vi.fn(),
    stop: vi.fn(),
    resume: vi.fn(),
    dismissEmptyNotice: vi.fn(),
    setAutoExtract: vi.fn(),
    enableAutoFromBanner: vi.fn(),
    dismissPromptForever: vi.fn(),
    closePromptForSession: vi.fn(),
    markStatus: vi.fn().mockResolvedValue(undefined),
    ...overrides,
  }
}

describe('ActionItems UI', () => {
  it('shows the first-hit guidance banner and wires Enable / Never / Close actions', async () => {
    const state = makeState({ showPromptBanner: true, phase: 'idle' })
    render(<ActionItemsPromptBanner state={state} />)
    await userEvent.click(screen.getByRole('button', { name: '启用' }))
    expect(state.enableAutoFromBanner).toHaveBeenCalledOnce()
    await userEvent.click(screen.getByRole('button', { name: '不再提示' }))
    expect(state.dismissPromptForever).toHaveBeenCalledOnce()
    await userEvent.click(screen.getByRole('button', { name: '关闭提示' }))
    expect(state.closePromptForSession).toHaveBeenCalledOnce()
  })

  it('merges L0 and L2 items, folds extras beyond two, and opens/dismisses items', async () => {
    const client = testClient()
    const items = [
      makeItem({ id: 'ics:1', source: 'ics', title: 'Kickoff' }),
      makeItem({ id: 'ai:1', source: 'ai', title: 'Design Sync' }),
      makeItem({ id: 'ai:2', source: 'ai', kind: 'task', title: 'Sign NDA', startAt: null }),
    ]
    const state = makeState({ visibleItems: items })
    render(<ActionItemsBar state={state} client={client} onOpenSettings={vi.fn()} />)
    expect(screen.getByText('Kickoff')).toBeInTheDocument()
    expect(screen.getByText('Design Sync')).toBeInTheDocument()
    expect(screen.queryByText('Sign NDA')).not.toBeInTheDocument()

    await userEvent.click(screen.getByRole('button', { name: /展开全部/ }))
    expect(screen.getByText('Sign NDA')).toBeInTheDocument()

    await userEvent.click(screen.getAllByRole('button', { name: '添加' })[0])
    expect(state.openItem).toHaveBeenCalledWith(items[0])
  })

  it('allows switching between Event and Task in the confirmation dialog and calls client', async () => {
    const client = testClient()
    const onAdded = vi.fn()
    const onClose = vi.fn()
    render(
      <ActionItemDialog
        client={client}
        emailSummary={SAMPLE_SUMMARY}
        item={makeItem()}
        onClose={onClose}
        onAdded={onAdded}
      />,
    )
    expect(screen.getByRole('button', { name: '添加到系统日历' })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '保存 .ics' })).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole('tab', { name: '待办' }))
    await userEvent.click(screen.getByRole('button', { name: '复制待办文本' }))
    expect(client.copyActionItemText).toHaveBeenCalledOnce()
    expect(onAdded).toHaveBeenCalledWith('ai:event:2026-12-03:abc')
  })

  it('shows a gentle empty-result notice with re-extract and close actions', async () => {
    const client = testClient()
    const state = makeState({ showEmptyNotice: true, visibleItems: [] })
    render(<ActionItemsBar state={state} client={client} onOpenSettings={vi.fn()} />)
    expect(screen.getByText('未在这封邮件中发现日程或待办。')).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '重新提取' }))
    expect(state.extract).toHaveBeenCalledWith(true)
    await userEvent.click(screen.getByRole('button', { name: '关闭提示' }))
    expect(state.dismissEmptyNotice).toHaveBeenCalledOnce()
  })
})
