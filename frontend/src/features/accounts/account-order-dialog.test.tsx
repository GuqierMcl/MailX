import { act, fireEvent, render, screen, waitFor, within } from '@/test/render'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { AccountsDialog } from './accounts-dialog'
import { exampleAccount, testClient } from './test-fixtures'
import { MailxClientError } from '@/lib/mailx-client'
import { i18n } from '@/i18n'
import type { Account } from '@/generated/accounts'
import { toast } from '@/components/ui/toast'

vi.mock('@/components/ui/toast', () => ({ toast: { error: vi.fn(), success: vi.fn(), info: vi.fn(), close: vi.fn() } }))

const first = exampleAccount()
const second = exampleAccount({ id: 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', displayName: '个人邮箱' })
const rows = [first, second]
const ResizeObserver = globalThis.ResizeObserver

// Give the real dnd-kit keyboard sensor the geometry jsdom does not provide.
beforeEach(() => {
  Object.defineProperty(document, 'getAnimations', { configurable: true, value: () => [] })
  Object.defineProperty(Element.prototype, 'animate', {
    configurable: true,
    value: () => ({ finished: Promise.resolve(), cancel() {} }),
  })
  vi.spyOn(Element.prototype, 'getBoundingClientRect').mockImplementation(function (this: Element) {
    const row = this.closest('[data-slot="sidebar-menu-item"]')
    if (!row) return DOMRect.fromRect({ x: 0, y: 0, width: 1024, height: 768 })
    const index = [...row.parentElement!.children].indexOf(row)
    return DOMRect.fromRect({ x: 10, y: 100 + index * 32, width: 220, height: 32 })
  })
  vi.spyOn(Element.prototype, 'getClientRects').mockImplementation(function (this: Element) {
    return [this.getBoundingClientRect()] as unknown as DOMRectList
  })
  vi.spyOn(document.documentElement, 'clientWidth', 'get').mockReturnValue(1024)
  vi.spyOn(document.documentElement, 'clientHeight', 'get').mockReturnValue(768)
  vi.stubGlobal('requestIdleCallback', (callback: IdleRequestCallback) => setTimeout(() => callback({ didTimeout: false, timeRemaining: () => 50 }), 0))
  vi.stubGlobal('cancelIdleCallback', clearTimeout)
  vi.stubGlobal('ResizeObserver', ResizeObserver)
  vi.stubGlobal('IntersectionObserver', class {
    observe() {}
    unobserve() {}
    disconnect() {}
  })
})
afterEach(() => {
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
  Reflect.deleteProperty(document, 'getAnimations')
  Reflect.deleteProperty(Element.prototype, 'animate')
})

function setup(client = testClient(rows)) {
  const onClose = vi.fn()
  const target = {}
  const { rerender } = render(<AccountsDialog client={client} accounts={rows} target={target} syncStates={{}} onClose={onClose} onAccountsChanged={vi.fn()} onAddAccount={vi.fn()} />)
  const list = screen.getByRole('list', { name: '邮箱账号列表' })
  const handles = () => within(list).getAllByRole('button', { name: /调整 .* 的顺序/ })
  const changeAccounts = (accounts: Account[]) => rerender(<AccountsDialog client={client} accounts={accounts} target={target} syncStates={{}} onClose={onClose} onAccountsChanged={vi.fn()} onAddAccount={vi.fn()} />)
  return { client, onClose, list, handles, changeAccounts }
}

async function lift(handle: HTMLElement) {
  await waitFor(() => expect(handle).toBeEnabled())
  await waitFor(() => expect(handle).toHaveAttribute('aria-disabled', 'false'))
  act(() => handle.focus())
  fireEvent.keyDown(handle, { key: ' ', code: 'Space' })
  await waitFor(() => expect(handle).toHaveAttribute('aria-pressed', 'true'))
}

async function moveDown(handle: HTMLElement) {
  fireEvent.keyDown(handle, { key: 'ArrowDown', code: 'ArrowDown' })
  expect(await screen.findByText('工作邮箱 已移到第 2 项，共 2 项。')).toBeInTheDocument()
}

describe('account order in the dialog', () => {
  it('saves one keyboard drop, retains the selected profile and restores handle focus', async () => {
    const { client, handles } = setup()
    const name = await screen.findByLabelText('账号名称')
    fireEvent.change(name, { target: { value: '正在编辑' } })
    const handle = handles()[0]
    await lift(handle)
    await moveDown(handle)
    expect(client.updateSettings).not.toHaveBeenCalled()
    fireEvent.keyDown(handle, { key: ' ', code: 'Space' })
    await waitFor(() => expect(client.updateSettings).toHaveBeenCalledExactlyOnceWith({ layout: { accountOrder: [second.id, first.id] } }))
    expect(handles()[0]).toHaveAccessibleName('调整 个人邮箱 的顺序')
    expect(screen.getByLabelText('账号名称')).toBe(name)
    expect(name).toHaveValue('正在编辑')
    await waitFor(() => expect(handle).toHaveFocus())
  })

  it('cancels with Escape without saving or closing the dialog', async () => {
    const { client, onClose, handles } = setup()
    const handle = handles()[0]
    await lift(handle)
    await moveDown(handle)
    fireEvent.keyDown(handle, { key: 'Escape', code: 'Escape' })
    await waitFor(() => expect(handle).toHaveAttribute('aria-pressed', 'false'))
    expect(client.updateSettings).not.toHaveBeenCalled()
    expect(onClose).not.toHaveBeenCalled()
    expect(handles()[0]).toBe(handle)
  })

  it('rolls back the display order and reports a failed save with a toast', async () => {
    const client = testClient(rows)
    client.updateSettings.mockRejectedValue(new MailxClientError({ code: 'storageUnavailable', field: null, message: '' }))
    const { handles } = setup(client)
    const handle = handles()[0]
    await lift(handle)
    await moveDown(handle)
    fireEvent.keyDown(handle, { key: 'Enter', code: 'Enter' })
    await waitFor(() => expect(toast.error).toHaveBeenCalledWith('账号顺序未保存', { description: expect.any(String) }))
    await waitFor(() => expect(handles()[0]).toBe(handle))
    expect(handle).toBeEnabled()
  })

  it('keeps focus while saving and prevents another keyboard drag until the save settles', async () => {
    const client = testClient(rows)
    const save = client.updateSettings.getMockImplementation()!
    let release!: () => void
    const pending = new Promise<void>((resolve) => { release = resolve })
    client.updateSettings.mockImplementation(async (patch) => { await pending; return save(patch) })
    const { handles } = setup(client)
    const handle = handles()[0]
    await lift(handle)
    await moveDown(handle)
    fireEvent.keyDown(handle, { key: 'Enter', code: 'Enter' })
    await waitFor(() => expect(client.updateSettings).toHaveBeenCalledTimes(1))
    await waitFor(() => expect(handle).toHaveFocus())
    expect(handle).toBeEnabled()
    expect(handle).toHaveAttribute('aria-disabled', 'true')
    fireEvent.keyDown(handle, { key: ' ', code: 'Space' })
    fireEvent.keyDown(handle, { key: 'ArrowUp', code: 'ArrowUp' })
    fireEvent.keyDown(handle, { key: 'Enter', code: 'Enter' })
    expect(handle).toHaveAttribute('aria-pressed', 'false')
    expect(client.updateSettings).toHaveBeenCalledTimes(1)

    await act(async () => { release(); await pending })
    await waitFor(() => expect(handle).toHaveAttribute('aria-disabled', 'false'))
    expect(handle).toHaveFocus()
    expect(client.updateSettings).toHaveBeenCalledTimes(1)
  })

  it('updates keyboard instructions and announcements when locale or accounts change', async () => {
    const { client, handles, changeAccounts } = setup()
    const handle = handles()[0]
    await waitFor(() => expect(handle).toHaveAttribute('aria-disabled', 'false'))
    expect(handle).toHaveAccessibleDescription('按空格或回车拿起账号，使用方向键调整位置，再按空格或回车放下，按 Esc 取消。')
    await act(async () => { await i18n.changeLanguage('en') })
    await waitFor(() => expect(handle).toHaveAccessibleDescription('Press Space or Enter to pick up an account, use the arrow keys to move it, then press Space or Enter to drop it. Press Escape to cancel.'))

    const renamed = { ...first, displayName: 'Renamed account' }
    const added = exampleAccount({ id: 'cccccccc-cccc-4ccc-8ccc-cccccccccccc', displayName: 'New account' })
    changeAccounts([renamed, second, added])
    expect(handle).toHaveAccessibleName('Reorder Renamed account')
    await lift(handle)
    fireEvent.keyDown(handle, { key: 'ArrowDown', code: 'ArrowDown' })
    expect(await screen.findByText('Renamed account moved to position 2 of 3.')).toBeInTheDocument()
    fireEvent.keyDown(handle, { key: 'Escape', code: 'Escape' })
    await waitFor(() => expect(handle).toHaveAttribute('aria-pressed', 'false'))
    expect(client.updateSettings).not.toHaveBeenCalled()
  })
})
