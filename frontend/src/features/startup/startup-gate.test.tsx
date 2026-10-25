import { act, render, screen, waitFor } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { StartupGate } from './startup-gate'
import { testClient } from '@/features/accounts/test-fixtures'
import type { MailxClient } from '@/lib/mailx-client'

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason?: unknown) => void
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise
    reject = rejectPromise
  })
  return { promise, resolve, reject }
}

describe('startup gate', () => {
  it('does not query or emulate a host when rendered in a browser', () => {
    const client = { ...testClient(), available: false }
    render(<StartupGate client={client} />)
    expect(screen.getByText('桌面能力不可用')).toBeInTheDocument()
    expect(client.getStartupStatus).not.toHaveBeenCalled()
    expect(client.listAccounts).not.toHaveBeenCalled()
  })

  it('keeps account UI unmounted until the host reports ready', async () => {
    const pending = deferred<Awaited<ReturnType<MailxClient['getStartupStatus']>>>()
    const client = testClient()
    client.getStartupStatus.mockReturnValueOnce(pending.promise)
    render(<StartupGate client={client} />)

    expect(screen.getByRole('status', { name: '正在初始化本地数据' })).toBeInTheDocument()
    expect(client.listAccounts).not.toHaveBeenCalled()
    await act(async () => { pending.resolve({ state: 'ready', revision: 1 }) })
    await waitFor(() => expect(screen.getByText('还没有邮箱账号')).toBeInTheDocument())
    expect(client.listAccounts).toHaveBeenCalledTimes(1)
  })

  it('polls an initializing host and stops after ready', async () => {
    const client = testClient([], { state: 'initializing', revision: 1 })
    client.getStartupStatus
      .mockResolvedValueOnce({ state: 'initializing', revision: 1 })
      .mockResolvedValueOnce({ state: 'ready', revision: 2 })
    render(<StartupGate client={client} />)
    await waitFor(() => expect(client.getStartupStatus).toHaveBeenCalledTimes(1))
    await new Promise((resolve) => setTimeout(resolve, 550))
    await waitFor(() => expect(client.getStartupStatus).toHaveBeenCalledTimes(2))
    expect(await screen.findByText('还没有邮箱账号')).toBeInTheDocument()
    await new Promise((resolve) => setTimeout(resolve, 550))
    expect(client.getStartupStatus).toHaveBeenCalledTimes(2)
  })

  it('does not repeat a retry when its response is lost', async () => {
    const client = testClient([], { state: 'failed', revision: 1, canRetry: true, error: { code: 'storageUnavailable', message: '本地数据暂时不可用。', field: null } })
    client.getStartupStatus
      .mockResolvedValueOnce({ state: 'failed', revision: 1, canRetry: true, error: { code: 'storageUnavailable', message: '本地数据暂时不可用。', field: null } })
      .mockResolvedValueOnce({ state: 'failed', revision: 2, canRetry: true, error: { code: 'storageUnavailable', message: '本地数据暂时不可用。', field: null } })
    client.retryStartup.mockRejectedValueOnce(new Error('transport lost'))
    render(<StartupGate client={client} />)
    await userEvent.click(await screen.findByRole('button', { name: '重试初始化' }))
    await waitFor(() => expect(client.getStartupStatus).toHaveBeenCalledTimes(2))
    expect(client.retryStartup).toHaveBeenCalledTimes(1)
    expect(screen.getByRole('button', { name: '重试初始化' })).toBeInTheDocument()
  })

  it('drops a stale lower revision returned by a retry', async () => {
    const client = testClient([], { state: 'failed', revision: 3, canRetry: true, error: { code: 'storageUnavailable', message: '当前错误。', field: null } })
    client.getStartupStatus.mockResolvedValueOnce({ state: 'failed', revision: 3, canRetry: true, error: { code: 'storageUnavailable', message: '当前错误。', field: null } })
    client.retryStartup.mockResolvedValueOnce({ state: 'failed', revision: 2, canRetry: true, error: { code: 'internal', message: '旧错误。', field: null } })
    render(<StartupGate client={client} />)
    await userEvent.click(await screen.findByRole('button', { name: '重试初始化' }))
    await waitFor(() => expect(client.retryStartup).toHaveBeenCalledTimes(1))
    expect(screen.getByText('无法读写本地数据，请检查可用空间与访问权限。')).toBeInTheDocument()
    expect(screen.queryByText('操作未完成，请重试。')).not.toBeInTheDocument()
  })
})
