import { act, fireEvent, render, screen, waitFor } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { i18n } from '@/i18n'
import { useMutation } from '@tanstack/react-query'
import { testClient } from '@/features/accounts/test-fixtures'
import { useDebouncedLayoutSave } from '@/features/settings/settings-queries'
import { SettingsDialog } from '@/features/settings/settings-dialog'
import type { MailxClient } from '@/lib/mailx-client'
import type { LifecycleClient, LifecycleSnapshot } from './lifecycle-client'
import { LifecycleProvider } from './lifecycle-provider'

function fixture(patch: Partial<LifecycleSnapshot> = {}) {
  let snapshot: LifecycleSnapshot = { revision: 1, platform: 'windows', backgroundAvailable: true, closeRequestId: 1, quitRequestId: null, quitting: false, quitTimedOut: false, ...patch }
  let listener: ((snapshot: LifecycleSnapshot) => void) | undefined
  const client = {
    getSnapshot: vi.fn<LifecycleClient['getSnapshot']>().mockImplementation(async () => snapshot),
    onSnapshot: vi.fn<LifecycleClient['onSnapshot']>().mockImplementation(async (next) => { listener = next; return () => { listener = undefined } }),
    resolveClose: vi.fn<LifecycleClient['resolveClose']>().mockResolvedValue(undefined),
    quit: vi.fn<LifecycleClient['quit']>().mockResolvedValue(undefined),
    cancelQuit: vi.fn<LifecycleClient['cancelQuit']>().mockResolvedValue(undefined),
    confirmQuit: vi.fn<LifecycleClient['confirmQuit']>().mockResolvedValue(undefined),
    onDesktopAction: vi.fn<LifecycleClient['onDesktopAction']>().mockResolvedValue(() => undefined),
    setDesktopReady: vi.fn<LifecycleClient['setDesktopReady']>().mockResolvedValue(undefined),
  }
  const publish = (next: Partial<LifecycleSnapshot>) => act(() => {
    snapshot = { ...snapshot, ...next, revision: snapshot.revision + 1 }
    listener?.(snapshot)
  })
  return { client, publish }
}

describe('main window close choice', () => {
  it('asks without remembering, and resets the checkbox on the next close request', async () => {
    const { client, publish } = fixture()
    render(<LifecycleProvider client={client}><p>Main window</p></LifecycleProvider>)
    expect(await screen.findByRole('dialog', { name: '关闭 MailX 后如何处理？' })).toBeInTheDocument()
    expect(screen.getByRole('checkbox', { name: '记住我的选择，不再询问' })).not.toBeChecked()
    await userEvent.click(screen.getByRole('button', { name: '最小化到系统托盘' }))
    expect(client.resolveClose).toHaveBeenCalledWith({ requestId: 1, action: 'background', remember: false })
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
    publish({ closeRequestId: 2 })
    await userEvent.click(await screen.findByRole('checkbox'))
    await userEvent.click(screen.getByRole('button', { name: '退出应用' }))
    expect(client.resolveClose).toHaveBeenLastCalledWith({ requestId: 2, action: 'quit', remember: true })
    publish({ closeRequestId: 3 })
    expect(await screen.findByRole('checkbox')).not.toBeChecked()
  })

  it('Escape cancels the close and never remembers the checked choice', async () => {
    const { client } = fixture()
    render(<LifecycleProvider client={client}><p>Main window</p></LifecycleProvider>)
    await userEvent.click(await screen.findByRole('checkbox'))
    await userEvent.keyboard('{Escape}')
    expect(client.resolveClose).toHaveBeenCalledWith({ requestId: 1, action: 'cancel', remember: false })
  })

  it('uses macOS window language without suggesting that a menu bar icon is required', async () => {
    const { client } = fixture({ platform: 'macos' })
    render(<LifecycleProvider client={client}><p>Main window</p></LifecycleProvider>)
    expect(await screen.findByRole('button', { name: '关闭窗口并继续运行' })).toBeInTheDocument()
    expect(screen.getByText(/可从 Dock 重新打开窗口/)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '最小化到系统托盘' })).not.toBeInTheDocument()
  })

  it('cannot hide to an unavailable tray, including a capability lost while asking', async () => {
    const { client, publish } = fixture({ platform: 'linux' })
    render(<LifecycleProvider client={client}><p>Main window</p></LifecycleProvider>)
    await screen.findByRole('button', { name: '最小化到系统托盘' })
    publish({ backgroundAvailable: false })
    expect(screen.queryByRole('button', { name: '最小化到系统托盘' })).not.toBeInTheDocument()
    expect(screen.getByText(/当前无法使用系统托盘/)).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '取消' }))
    expect(client.resolveClose).toHaveBeenCalledWith({ requestId: 1, action: 'cancel', remember: false })
  })

  it('keeps a failed choice open, hides raw transport errors and prevents double submission', async () => {
    const { client } = fixture()
    let fail!: (error: Error) => void
    client.resolveClose.mockImplementationOnce(() => new Promise((_resolve, reject) => { fail = reject }))
    render(<LifecycleProvider client={client}><p>Main window</p></LifecycleProvider>)
    const button = await screen.findByRole('button', { name: '退出应用' })
    fireEvent.click(button)
    fireEvent.click(button)
    expect(client.resolveClose).toHaveBeenCalledTimes(1)
    expect(button).toBeDisabled()
    await act(async () => fail(new Error('/private/secret/path')))
    expect(await screen.findByRole('alert')).not.toHaveTextContent('/private/secret/path')
    expect(button).not.toBeDisabled()
    await userEvent.click(button)
    expect(client.resolveClose).toHaveBeenCalledTimes(2)
  })

  it('does not let an older initial read overwrite a newer close event', async () => {
    const { client, publish } = fixture({ closeRequestId: null })
    let complete!: (snapshot: LifecycleSnapshot) => void
    client.getSnapshot.mockImplementationOnce(() => new Promise((resolve) => { complete = resolve }))
    render(<LifecycleProvider client={client}><p>Main window</p></LifecycleProvider>)
    await waitFor(() => expect(client.getSnapshot).toHaveBeenCalled())
    publish({ closeRequestId: 9 })
    await act(async () => complete({ revision: 1, platform: 'windows', backgroundAvailable: true, closeRequestId: null, quitRequestId: null, quitting: false, quitTimedOut: false }))
    await userEvent.click(await screen.findByRole('button', { name: '取消' }))
    expect(client.resolveClose).toHaveBeenCalledWith({ requestId: 9, action: 'cancel', remember: false })
  })

  it('shows quit progress and allows cancelling the quit', async () => {
    const { client, publish } = fixture({ closeRequestId: null, quitRequestId: null, quitting: true })
    render(<LifecycleProvider client={client}><input aria-label="Editor" /></LifecycleProvider>)
    await userEvent.click(await screen.findByRole('button', { name: '取消退出' }))
    expect(client.cancelQuit).toHaveBeenCalledOnce()
    expect(screen.getByRole('dialog', { name: '正在退出 MailX' })).toBeInTheDocument()
    publish({ quitting: false, quitTimedOut: true })
    expect(screen.getByRole('dialog', { name: '退出已取消' })).toHaveTextContent('退出已取消')
    await userEvent.click(screen.getByRole('button', { name: '知道了' }))
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument()
  })

  it('provides English text for the same close choices', async () => {
    await i18n.changeLanguage('en')
    const { client } = fixture()
    render(<LifecycleProvider client={client}><p>Main window</p></LifecycleProvider>)
    expect(await screen.findByRole('dialog', { name: 'What should happen when you close MailX?' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Minimize to system tray' })).toBeInTheDocument()
    expect(screen.getByRole('checkbox', { name: 'Remember my choice and do not ask again' })).not.toBeChecked()
  })
})

function LayoutEditor({ client }: { client: MailxClient }) {
  const save = useDebouncedLayoutSave(client)
  return <button onClick={() => save({ listWidth: 630 })}>Resize list</button>
}

function MutationEditor({ save }: { save: () => Promise<void> }) {
  const mutation = useMutation({ mutationFn: save })
  return <button onClick={() => mutation.mutate()}>Save preference</button>
}

describe('main window quit acknowledgement', () => {
  it('flushes an immediate layout change and waits for persistence before acknowledging', async () => {
    const { client, publish } = fixture({ closeRequestId: null })
    const mail = testClient()
    const save = mail.updateSettings.getMockImplementation()!
    let finish!: () => void
    mail.updateSettings.mockImplementationOnce(async (patch) => {
      await new Promise<void>((resolve) => { finish = resolve })
      return save(patch)
    })
    render(<LifecycleProvider client={client}><LayoutEditor client={mail} /></LifecycleProvider>)
    await waitFor(() => expect(client.getSnapshot).toHaveBeenCalled())
    fireEvent.click(screen.getByRole('button', { name: 'Resize list' }))
    publish({ quitting: true, quitRequestId: 11 })
    await waitFor(() => expect(mail.updateSettings).toHaveBeenCalled())
    expect(mail.updateSettings).toHaveBeenCalledWith({ layout: expect.objectContaining({ listWidth: 630 }) })
    expect(screen.getByText('Resize list').closest('[data-main-interaction]')).toHaveAttribute('inert')
    expect(client.confirmQuit).not.toHaveBeenCalled()
    await act(async () => finish())
    await waitFor(() => expect(client.confirmQuit).toHaveBeenCalledWith(11))
  })

  it('also waits for a preference mutation already in progress', async () => {
    const { client, publish } = fixture({ closeRequestId: null })
    let finish!: () => void
    const save = vi.fn(() => new Promise<void>((resolve) => { finish = resolve }))
    render(<LifecycleProvider client={client}><MutationEditor save={save} /></LifecycleProvider>)
    await waitFor(() => expect(client.getSnapshot).toHaveBeenCalled())
    fireEvent.click(screen.getByRole('button', { name: 'Save preference' }))
    await waitFor(() => expect(save).toHaveBeenCalled())
    publish({ quitting: true, quitRequestId: 12 })
    await screen.findByRole('button', { name: '取消退出' })
    expect(client.confirmQuit).not.toHaveBeenCalled()
    await act(async () => finish())
    await waitFor(() => expect(client.confirmQuit).toHaveBeenCalledWith(12))
  })

  it('cancels a failed save and retains the layout for a later quit retry', async () => {
    const { client, publish } = fixture({ closeRequestId: null })
    const mail = testClient()
    mail.updateSettings.mockRejectedValueOnce(new Error('/private/save-failed'))
    render(<LifecycleProvider client={client}><LayoutEditor client={mail} /></LifecycleProvider>)
    await waitFor(() => expect(client.getSnapshot).toHaveBeenCalled())
    fireEvent.click(screen.getByRole('button', { name: 'Resize list' }))
    publish({ quitting: true, quitRequestId: 13 })
    await waitFor(() => expect(client.cancelQuit).toHaveBeenCalledWith(13))
    expect(client.confirmQuit).not.toHaveBeenCalled()
    expect(await screen.findByRole('dialog', { name: '无法完成退出前的保存' })).not.toHaveTextContent('/private/save-failed')
    publish({ quitting: false, quitRequestId: null })
    publish({ quitting: true, quitRequestId: 14 })
    await waitFor(() => expect(client.confirmQuit).toHaveBeenCalledWith(14))
    expect(mail.updateSettings).toHaveBeenCalledTimes(2)
    expect(mail.updateSettings).toHaveBeenLastCalledWith({ layout: expect.objectContaining({ listWidth: 630 }) })
  })

  it.each([null, 22])('does not acknowledge a cancelled generation when the next generation is %s', async (next) => {
    const { client, publish } = fixture({ closeRequestId: null })
    const mail = testClient()
    const save = mail.updateSettings.getMockImplementation()!
    let finish!: () => void
    mail.updateSettings.mockImplementationOnce(async (patch) => {
      await new Promise<void>((resolve) => { finish = resolve })
      return save(patch)
    })
    render(<LifecycleProvider client={client}><LayoutEditor client={mail} /></LifecycleProvider>)
    await waitFor(() => expect(client.getSnapshot).toHaveBeenCalled())
    fireEvent.click(screen.getByRole('button', { name: 'Resize list' }))
    publish({ quitting: true, quitRequestId: 21 })
    await waitFor(() => expect(mail.updateSettings).toHaveBeenCalled())
    publish({ quitting: false, quitRequestId: null })
    if (next !== null) publish({ quitting: true, quitRequestId: next })
    await act(async () => finish())
    if (next !== null) await waitFor(() => expect(client.confirmQuit).toHaveBeenCalledWith(next))
    expect(client.confirmQuit).not.toHaveBeenCalledWith(21)
    if (next === null) expect(client.confirmQuit).not.toHaveBeenCalled()
  })

  it('keeps Cancel quit reachable above an already open General settings modal', async () => {
    const { client, publish } = fixture({ closeRequestId: null })
    client.quit.mockImplementation(async () => { publish({ quitting: true, quitRequestId: 31 }) })
    render(<LifecycleProvider client={client}><SettingsDialog client={testClient()} target={{ section: 'general' }} onClose={() => undefined} /></LifecycleProvider>)
    await userEvent.click(await screen.findByRole('button', { name: '退出 MailX' }))
    const progress = await screen.findByRole('dialog', { name: '正在退出 MailX' })
    expect(progress).toHaveAttribute('data-quit-controls')
    await userEvent.click(screen.getByRole('button', { name: '取消退出' }))
    expect(client.cancelQuit).toHaveBeenCalledWith(31)
  })
})
