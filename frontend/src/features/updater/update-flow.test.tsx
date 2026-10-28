import { describe, expect, it, vi } from 'vitest'
import { render, screen, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { Toaster } from '@/components/ui/toast'
import { testClient } from '@/features/accounts/test-fixtures'
import { withTitleBar } from '@/test/title-bar'
import { UpdaterProvider } from './updater-provider'
import { useUpdater } from './updater-context'
import { UpdateIndicator } from './update-indicator'
import { UpdateDialog } from './update-dialog'
import type { UpdateSnapshot } from './types'

const available: UpdateSnapshot = {
  canInstall: true, phase: 'available', version: '0.2.0', current: '0.1.0',
  notes: '# What is new\n\n- **Faster** sync\n- A [link](https://example.com/x)', date: 1767254400,
}

/** The title-bar update button's label; it sits behind a modal while one is open. */
const titleBarLabel = () => document.querySelector('[data-slot="title-bar-trailing"] button')?.getAttribute('aria-label') ?? null

function setup(initial: UpdateSnapshot, options: { canInstall?: boolean } = {}) {
  const client = testClient()
  client.getUpdateState.mockResolvedValue(options.canInstall === undefined ? initial : { ...initial, canInstall: options.canInstall } as UpdateSnapshot)
  const state: { emit: (next: UpdateSnapshot) => void } = { emit: () => undefined }
  client.onUpdateState.mockImplementation((listener) => { state.emit = listener; return () => undefined })
  function Harness() {
    const { snapshot, openDialog } = useUpdater()
    return <><UpdateIndicator snapshot={snapshot} onOpen={openDialog} /></>
  }
  render(withTitleBar(<><Toaster /><UpdaterProvider client={client}><Harness /></UpdaterProvider></>))
  return { client, emit: (next: UpdateSnapshot) => state.emit(next) }
}

describe('the update flow', () => {
  it('downloads, installs and restarts in order', async () => {
    const { client, emit } = setup(available)
    await userEvent.click(await screen.findByRole('button', { name: 'MailX 0.2.0 可用' }))
    expect(screen.getByRole('dialog', { name: 'MailX 0.2.0' })).toBeInTheDocument()
    expect(screen.getByText('0.1.0 → 0.2.0')).toBeInTheDocument()
    // Notes are elements, including a link that goes back to the host.
    expect(screen.getByText('Faster').closest('strong')).not.toBeNull()
    await userEvent.click(screen.getByRole('button', { name: 'link' }))
    expect(client.openUpdateLink).toHaveBeenCalledWith('https://example.com/x')

    await userEvent.click(screen.getByRole('button', { name: '下载更新' }))
    expect(client.downloadUpdate).toHaveBeenCalledOnce()
    // Progress comes from the host, not from the button.
    emit({ ...available, phase: 'downloading', downloaded: 45, total: 100 })
    expect(await screen.findByRole('progressbar')).toHaveAttribute('aria-valuenow', '45')
    expect(screen.getByText('Faster')).toBeInTheDocument()
    expect(screen.getByText('0.1.0 → 0.2.0')).toBeInTheDocument()
    expect(screen.getByText(/^发布于 /)).toBeInTheDocument()
    expect(screen.queryByText('这个版本没有提供更新说明。')).not.toBeInTheDocument()
    // While it downloads, the title-bar button turns into a progress ring. The
    // modal hides the title bar from the accessibility tree, so read it directly.
    expect(titleBarLabel()).toBe('正在下载 0.2.0（45%）')
    await userEvent.click(screen.getByRole('button', { name: '后台继续' }))
    await userEvent.click(await screen.findByRole('button', { name: '正在下载 0.2.0（45%）' }))
    expect(screen.getByText('Faster')).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '取消下载' }))
    expect(client.cancelUpdate).toHaveBeenCalledOnce()

    emit(available)
    await userEvent.click(await screen.findByRole('button', { name: '下载更新' }))
    expect(client.downloadUpdate).toHaveBeenCalledTimes(2)
    emit({ ...available, phase: 'downloading', downloaded: 100, total: 100 })
    emit({ ...available, phase: 'ready' })
    expect(await screen.findByText('Faster')).toBeInTheDocument()
    expect(screen.getByText('0.1.0 → 0.2.0')).toBeInTheDocument()
    expect(screen.queryByText('这个版本没有提供更新说明。')).not.toBeInTheDocument()
    await userEvent.click(await screen.findByRole('button', { name: '立即重启' }))
    expect(client.restartToUpdate).toHaveBeenCalledOnce()
  })

  it('refuses to restart while mail is being sent, with the reason', async () => {
    const { client } = setup({ ...available, phase: 'ready' })
    const { MailxClientError } = await import('@/lib/mailx-client')
    client.restartToUpdate.mockRejectedValueOnce(new MailxClientError({ code: 'alreadySending', message: '', field: null }))
    // The ready state is already the title-bar button; the modal follows a click.
    await userEvent.click(await screen.findByRole('button', { name: '重启以更新到 0.2.0' }))
    const dialog = await screen.findByRole('dialog', { name: 'MailX 0.2.0' })
    await userEvent.click(within(dialog).getByRole('button', { name: '立即重启' }))
    expect(await screen.findByText('有邮件正在发送，请等发送完成后再重启。')).toBeInTheDocument()
    expect(client.restartToUpdate).toHaveBeenCalledOnce()
  })

  it('offers a retry when a download failed', async () => {
    const { client, emit } = setup(available)
    await userEvent.click(await screen.findByRole('button', { name: 'MailX 0.2.0 可用' }))
    emit({ ...available, phase: 'failed', stage: 'download' })
    expect(await screen.findByText('下载没有完成，请重试。')).toBeInTheDocument()
    expect(screen.getByText('Faster')).toBeInTheDocument()
    expect(screen.getByText('0.1.0 → 0.2.0')).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '重试' }))
    expect(client.downloadUpdate).toHaveBeenCalledOnce()
    expect(client.checkForUpdates).not.toHaveBeenCalled()
    emit({ ...available, phase: 'downloading', downloaded: 0, total: null })
    expect(await screen.findByRole('progressbar')).not.toHaveAttribute('aria-valuenow')
    expect(screen.getByText('Faster')).toBeInTheDocument()
    expect(screen.queryByText('这个版本没有提供更新说明。')).not.toBeInTheDocument()
  })

  it.each([
    { ...available, phase: 'downloading', downloaded: 45, total: 100 },
    { ...available, phase: 'ready' },
    { ...available, phase: 'failed', stage: 'download' },
  ] satisfies UpdateSnapshot[])('reads release details on the first mount in $phase', async (snapshot) => {
    if (snapshot.phase === 'failed') {
      render(<UpdateDialog client={testClient()} snapshot={snapshot} open onOpenChange={() => undefined} />)
    } else {
      setup(snapshot)
      const label = snapshot.phase === 'downloading' ? '正在下载 0.2.0（45%）' : '重启以更新到 0.2.0'
      await userEvent.click(await screen.findByRole('button', { name: label }))
    }
    expect(await screen.findByText('Faster')).toBeInTheDocument()
    expect(screen.getByText('0.1.0 → 0.2.0')).toBeInTheDocument()
    expect(screen.getByText(/^发布于 /)).toBeInTheDocument()
    expect(screen.queryByText('这个版本没有提供更新说明。')).not.toBeInTheDocument()
  })

  it('shows the fallback only when the target version has no release notes', async () => {
    const { emit } = setup(available)
    await userEvent.click(await screen.findByRole('button', { name: 'MailX 0.2.0 可用' }))
    expect(screen.getByText('Faster')).toBeInTheDocument()
    emit({ ...available, phase: 'downloading', version: '0.3.0', notes: null, date: null, downloaded: 0, total: null })
    expect(await screen.findByText('这个版本没有提供更新说明。')).toBeInTheDocument()
    expect(screen.queryByText('Faster')).not.toBeInTheDocument()
    expect(screen.queryByText(/^发布于 /)).not.toBeInTheDocument()
    expect(screen.getByText('0.1.0 → 0.3.0')).toBeInTheDocument()
  })

  it('only explains how to get the release when the edition cannot install', async () => {
    const { client } = setup({ ...available, canInstall: false } as UpdateSnapshot, { canInstall: false })
    await userEvent.click(await screen.findByRole('button', { name: 'MailX 0.2.0 可用' }))
    expect(screen.getByRole('dialog', { name: 'MailX 0.2.0' })).toBeInTheDocument()
    expect(screen.getByText('这个界面只能告诉你新版本的内容；请在 MailX 桌面应用中安装更新。')).toBeInTheDocument()
    for (const name of ['下载更新', '立即重启', '稍后（退出时安装）', '取消下载', '后台继续', '稍后']) {
      expect(screen.queryByRole('button', { name }), name).not.toBeInTheDocument()
    }
    expect(screen.queryByRole('progressbar')).not.toBeInTheDocument()
    expect(client.downloadUpdate).not.toHaveBeenCalled()
    expect(client.restartToUpdate).not.toHaveBeenCalled()
  })
})

describe('update notices', () => {
  it('announces a found version once per run and opens the modal on a manual check', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true })
    try {
      const client = testClient()
      client.getUpdateState.mockResolvedValue({ canInstall: true, phase: 'idle' })
      const state: { emit: (next: UpdateSnapshot) => void } = { emit: () => undefined }
      client.onUpdateState.mockImplementation((listener) => { state.emit = listener; return () => undefined })
      function Manual() {
        const { check } = useUpdater()
        return <button type="button" onClick={() => void check()}>check</button>
      }
      render(withTitleBar(<><Toaster /><UpdaterProvider client={client}><Manual /></UpdaterProvider></>))

      // The automatic check finds 0.2.0: a toast, not the modal.
      state.emit(available)
      expect(await screen.findByText('MailX 0.2.0 可用')).toBeInTheDocument()
      expect(screen.queryByRole('dialog', { name: 'MailX 0.2.0' })).not.toBeInTheDocument()
      // The same version again in the same run says nothing new.
      state.emit({ canInstall: true, phase: 'checking' })
      state.emit(available)
      expect(screen.getAllByText('MailX 0.2.0 可用')).toHaveLength(1)
      // Closing the toast leaves the title-bar button.
      await userEvent.click(screen.getByRole('button', { name: '查看更新' }))
      expect(screen.getByRole('dialog', { name: 'MailX 0.2.0' })).toBeInTheDocument()
    } finally {
      vi.useRealTimers()
    }
  })
})
