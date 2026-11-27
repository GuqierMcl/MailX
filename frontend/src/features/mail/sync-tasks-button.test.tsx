import { act, fireEvent, render, screen, waitFor } from '@/test/render'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { AccountBackfillTask } from '@/generated/accounts'
import { exampleAccount, testClient } from '@/features/accounts/test-fixtures'
import { errorText } from '@/i18n/errors'
import { withTitleBar } from '@/test/title-bar'
import { SyncTasksButton } from './sync-tasks-button'

function runningTask(overrides: Partial<AccountBackfillTask> = {}): AccountBackfillTask {
  return {
    accountId: exampleAccount().id,
    phase: 'running',
    targetDate: '2026-07-01',
    coveredSince: '2026-09-01',
    fetched: 40,
    total: 100,
    activeFolderName: '收件箱',
    failure: null,
    ...overrides,
  }
}

describe('SyncTasksButton', () => {
  afterEach(() => {
    vi.useRealTimers()
  })

  it('is hidden when there are no backfill tasks', () => {
    const client = testClient([exampleAccount()])
    render(withTitleBar(
      <SyncTasksButton client={client} accounts={[exampleAccount()]} tasks={[]} onTasksChange={vi.fn()} />,
    ))
    expect(screen.queryByRole('button', { name: '同步任务' })).not.toBeInTheDocument()
  })

  it('waits 1.5s before showing a quiet running task and uses no spinner animation', () => {
    vi.useFakeTimers()
    const client = testClient([exampleAccount()])
    const { container, rerender } = render(withTitleBar(
      <SyncTasksButton
        client={client}
        accounts={[exampleAccount()]}
        tasks={[runningTask()]}
        onTasksChange={vi.fn()}
      />,
    ))
    expect(screen.queryByRole('button', { name: '同步任务' })).not.toBeInTheDocument()

    act(() => {
      vi.advanceTimersByTime(1400)
    })
    expect(screen.queryByRole('button', { name: '同步任务' })).not.toBeInTheDocument()

    act(() => {
      vi.advanceTimersByTime(150)
    })
    expect(screen.getByRole('button', { name: '同步任务' })).toBeInTheDocument()
    expect(screen.queryByTestId('sync-tasks-dot')).not.toBeInTheDocument()
    expect(container.querySelector('.animate-spin')).toBeNull()

    rerender(withTitleBar(
      <SyncTasksButton client={client} accounts={[exampleAccount()]} tasks={[]} onTasksChange={vi.fn()} />,
    ))
    expect(screen.queryByRole('button', { name: '同步任务' })).not.toBeInTheDocument()
  })

  it('shows paused and failed tasks immediately with their status dots and actions', async () => {
    const account = exampleAccount()
    const client = testClient([account])
    const paused = runningTask({ phase: 'paused' })
    const nextAfterResume = runningTask({ phase: 'running' })
    client.controlBackfill.mockResolvedValue([nextAfterResume])
    const onTasksChange = vi.fn()

    const { rerender } = render(withTitleBar(
      <SyncTasksButton client={client} accounts={[account]} tasks={[paused]} onTasksChange={onTasksChange} />,
    ))
    const button = screen.getByRole('button', { name: '同步任务' })
    expect(screen.getByTestId('sync-tasks-dot')).toHaveClass('bg-amber-500')

    fireEvent.click(button)
    expect(await screen.findByText('工作邮箱 · 收件箱')).toBeInTheDocument()
    expect(screen.getByText('已同步 40 / 100 封')).toBeInTheDocument()
    expect(screen.getByText('已覆盖至 2026-09-01')).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: '继续' }))
    await waitFor(() => expect(client.controlBackfill).toHaveBeenCalledWith(account.id, 'resume'))
    await waitFor(() => expect(onTasksChange).toHaveBeenCalledWith([nextAfterResume]))

    const failed = runningTask({
      phase: 'failed',
      failure: { code: 'network', message: '无法连接到邮件服务器。', detail: null, willRetry: true },
    })
    rerender(withTitleBar(
      <SyncTasksButton client={client} accounts={[account]} tasks={[failed]} onTasksChange={onTasksChange} />,
    ))
    expect(screen.getByTestId('sync-tasks-dot')).toHaveClass('bg-destructive')
    expect(screen.getByText(errorText(failed.failure))).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '重试' })).toBeInTheDocument()

    fireEvent.click(screen.getByRole('button', { name: '取消' }))
    await waitFor(() => expect(client.controlBackfill).toHaveBeenCalledWith(account.id, 'cancel'))
  })

  it('updates the progress ring stroke offset as fetched/total advances', () => {
    const account = exampleAccount()
    const client = testClient([account])
    const { rerender } = render(withTitleBar(
      <SyncTasksButton
        client={client}
        accounts={[account]}
        tasks={[runningTask({ phase: 'paused', fetched: 20, total: 100 })]}
        onTasksChange={vi.fn()}
      />,
    ))
    const offsetAt20 = Number(screen.getByTestId('sync-tasks-ring-progress').getAttribute('stroke-dashoffset'))

    rerender(withTitleBar(
      <SyncTasksButton
        client={client}
        accounts={[account]}
        tasks={[runningTask({ phase: 'paused', fetched: 80, total: 100 })]}
        onTasksChange={vi.fn()}
      />,
    ))
    const offsetAt80 = Number(screen.getByTestId('sync-tasks-ring-progress').getAttribute('stroke-dashoffset'))

    expect(offsetAt80).toBeLessThan(offsetAt20)
  })
})
