import { act, fireEvent, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import * as tauri from '@tauri-apps/api/core'
import * as menuDesktop from '@/features/app-menu/desktop'
import { i18n } from '@/i18n'
import { useAppMenuCommands } from '@/features/app-menu/commands'
import { TitleBar, TitleBarActions, TitleBarSlotsProvider } from './title-bar'
import { createPreviewWindowHost, previewPlatformFromUrl, type WindowChrome, type WindowHost, type WindowState } from './window-host'

vi.mock('@tauri-apps/api/core', { spy: true })

const normal: WindowState = { maximized: false, fullscreen: false, focused: true }

function fakeHost(chrome: Partial<WindowChrome> & Pick<WindowChrome, 'platform'>) {
  let emit: (state: WindowState) => void = () => undefined
  const host: WindowHost = {
    getChrome: async () => ({ leading: [], trailing: [], state: normal, translucent: false, ...chrome }),
    perform: vi.fn(),
    onStateChange(listener) {
      emit = listener
      return () => undefined
    },
  }
  return { host, perform: host.perform as ReturnType<typeof vi.fn>, emit: (state: WindowState) => act(() => emit(state)) }
}

async function renderTitleBar(host: WindowHost) {
  render(
    <TitleBar host={host} title="MailX">
      <button type="button">搜索</button>
    </TitleBar>,
  )
  const header = screen.getByRole('banner')
  await screen.findByText('MailX')
  await vi.waitFor(() => expect(header.dataset.platform).toBeDefined())
  return header
}

describe('TitleBar', () => {
  it('opens the Windows application menu from the keyboard and runs a submenu command', async () => {
    const { host, perform } = fakeHost({ platform: 'windows', trailing: ['minimize', 'maximize', 'close'] })
    const user = userEvent.setup()
    await renderTitleBar(host)

    const trigger = screen.getByRole('button', { name: '应用菜单' })
    expect(trigger.nextElementSibling?.firstElementChild).toHaveAttribute('data-window-button', 'minimize')
    trigger.focus()
    await user.keyboard('{Enter}')
    const view = await screen.findByRole('menuitem', { name: '显示' })
    view.focus()
    await user.keyboard('{ArrowRight}')
    const fullscreen = await screen.findByRole('menuitem', { name: '进入全屏' })
    fullscreen.focus()
    await user.keyboard('{Enter}')
    expect(perform).toHaveBeenCalledWith('toggleFullscreen')
  })

  it('uses the focused editor for edit actions and returns focus to it', async () => {
    const { host } = fakeHost({ platform: 'windows' })
    const user = userEvent.setup()
    const execCommand = vi.fn(() => true)
    const original = Object.getOwnPropertyDescriptor(document, 'execCommand')
    Object.defineProperty(document, 'execCommand', { configurable: true, value: execCommand })
    try {
      render(<><TitleBar host={host} /><input aria-label="主题" defaultValue="Draft" /></>)
      const editor = screen.getByRole('textbox', { name: '主题' }) as HTMLInputElement
      const trigger = await screen.findByRole('button', { name: '应用菜单' })
      await user.click(editor)
      editor.setSelectionRange(0, 5)
      await user.click(trigger)
      await user.click(await screen.findByRole('menuitem', { name: '编辑' }))
      fireEvent.click(await screen.findByRole('menuitem', { name: /复制/ }))
      await vi.waitFor(() => expect(execCommand).toHaveBeenCalledWith('copy'))
      await vi.waitFor(() => expect(editor).toHaveFocus())
    } finally {
      if (original) Object.defineProperty(document, 'execCommand', original)
      else Reflect.deleteProperty(document, 'execCommand')
    }
  })

  it('executes the latest registered settings action with Ctrl+, in Windows preview', async () => {
    const host = createPreviewWindowHost('windows')
    const previous = vi.fn()
    const current = vi.fn()
    function Commands({ open, enabled = true }: { open: () => void; enabled?: boolean }) {
      useAppMenuCommands({ settings: { run: open, enabled } })
      return <TitleBar host={host} />
    }
    const { rerender } = render(<Commands open={previous} />)
    await screen.findByRole('button', { name: '应用菜单' })
    rerender(<Commands open={current} />)
    expect(fireEvent.keyDown(window, { key: ',', metaKey: true })).toBe(true)
    expect(fireEvent.keyDown(window, { key: ',', ctrlKey: true, metaKey: true })).toBe(true)
    expect(fireEvent.keyDown(window, { key: ',', ctrlKey: true })).toBe(false)
    rerender(<Commands open={current} enabled={false} />)
    expect(fireEvent.keyDown(window, { key: ',', ctrlKey: true })).toBe(true)
    expect(current).toHaveBeenCalledOnce()
    expect(previous).not.toHaveBeenCalled()
  })

  it('keeps browser shortcuts available on a page without a desktop host or platform preview', () => {
    const run = vi.fn()
    function Commands() {
      useAppMenuCommands({ settings: { run } })
      return <TitleBar host={null} />
    }
    render(<Commands />)
    expect(fireEvent.keyDown(window, { key: ',', ctrlKey: true })).toBe(true)
    expect(fireEvent.keyDown(window, { key: ',', metaKey: true })).toBe(true)
    expect(run).not.toHaveBeenCalled()
  })

  it('leaves Ctrl+Q, Ctrl+W and Ctrl+M in a Windows text field to the platform', async () => {
    const { host, perform } = fakeHost({ platform: 'windows' })
    const quit = vi.fn()
    function Commands() {
      useAppMenuCommands({ quit: { run: quit } })
      return <><TitleBar host={host} /><input aria-label="主题" /></>
    }
    const desktop = vi.spyOn(tauri, 'isTauri').mockReturnValue(true)
    const windows = vi.spyOn(menuDesktop, 'listMenuWindows').mockResolvedValue([])
    const events = vi.spyOn(menuDesktop, 'listenToMenuEvent').mockReturnValue(() => {})
    try {
      render(<Commands />)
      await screen.findByRole('button', { name: '应用菜单' })
      const input = screen.getByRole('textbox', { name: '主题' })
      input.focus()
      for (const key of ['q', 'w', 'm']) {
        expect(fireEvent.keyDown(input, { key, code: `Key${key.toUpperCase()}`, ctrlKey: true })).toBe(true)
      }
      expect(quit).not.toHaveBeenCalled()
      expect(perform).not.toHaveBeenCalled()
    } finally {
      events.mockRestore()
      windows.mockRestore()
      desktop.mockRestore()
    }
  })

  it('keeps an open dialog in control of the keyboard', async () => {
    const host = createPreviewWindowHost('macos')
    const open = vi.fn()
    function Commands({ modal }: { modal: boolean }) {
      useAppMenuCommands({ settings: { run: open } })
      return <><TitleBar host={host} />{modal && <div role="dialog" data-open="" />}</>
    }
    const { rerender } = render(<Commands modal />)
    await vi.waitFor(() => expect(screen.getByRole('banner')).toHaveAttribute('data-platform', 'macos'))
    expect(fireEvent.keyDown(window, { key: ',', metaKey: true })).toBe(true)
    expect(open).not.toHaveBeenCalled()
    rerender(<Commands modal={false} />)
    expect(fireEvent.keyDown(window, { key: ',', ctrlKey: true })).toBe(true)
    expect(fireEvent.keyDown(window, { key: ',', metaKey: true })).toBe(false)
    expect(open).toHaveBeenCalledOnce()
  })

  it('draws Windows caption buttons and follows window state', async () => {
    const { host, perform, emit } = fakeHost({ platform: 'windows', trailing: ['minimize', 'maximize', 'close'] })
    const header = await renderTitleBar(host)

    const buttons = header.querySelectorAll('[data-window-button]')
    expect([...buttons].map((button) => button.getAttribute('aria-label'))).toEqual(['最小化', '最大化', '关闭'])

    fireEvent.click(screen.getByRole('button', { name: '最大化' }))
    expect(perform).toHaveBeenCalledWith('toggleMaximize')

    emit({ ...normal, maximized: true, focused: false })
    expect(screen.getByRole('button', { name: '还原' })).toBeInTheDocument()
    expect(header).toHaveAttribute('data-focused', 'false')
  })

  it.each(['windows', 'linux'] as const)('updates %s button labels without a host state change', async (platform) => {
    const { host, perform } = fakeHost({ platform, trailing: ['minimize', 'maximize', 'close'] })
    const header = await renderTitleBar(host)
    const buttons = [...header.querySelectorAll('[data-window-button]')]
    expect(buttons.map((button) => button.getAttribute('aria-label'))).toEqual(['最小化', '最大化', '关闭'])

    await act(async () => { await i18n.changeLanguage('en') })

    expect([...header.querySelectorAll('[data-window-button]')]).toEqual(buttons)
    expect(buttons.map((button) => button.getAttribute('aria-label'))).toEqual(['Minimize', 'Maximize', 'Close'])
    expect(buttons.map((button) => button.getAttribute('title'))).toEqual(['Minimize', 'Maximize', 'Close'])
    expect(perform).not.toHaveBeenCalled()
    expect(header).toHaveAttribute('data-focused', 'true')
  })

  it('drags from empty areas but not from interactive content', async () => {
    const { host, perform } = fakeHost({ platform: 'windows', trailing: ['close'] })
    const header = await renderTitleBar(host)

    fireEvent.mouseDown(screen.getByRole('button', { name: '搜索' }), { button: 0, detail: 1 })
    fireEvent.mouseDown(screen.getByRole('button', { name: '关闭' }), { button: 0, detail: 1 })
    fireEvent.mouseDown(header, { button: 2, detail: 1 })
    expect(perform).not.toHaveBeenCalled()

    fireEvent.mouseDown(screen.getByText('MailX'), { button: 0, detail: 1 })
    expect(perform).toHaveBeenLastCalledWith('startDragging')

    fireEvent.mouseDown(header, { button: 0, detail: 2 })
    expect(perform).toHaveBeenLastCalledWith('titleBarDoubleClick')
  })

  it('leaves macOS traffic lights to the system and acts on a released double click', async () => {
    const { host, perform, emit } = fakeHost({ platform: 'macos' })
    const header = await renderTitleBar(host)

    expect(header.querySelector('[data-window-button]')).toBeNull()
    expect(header.querySelector('[data-slot="title-bar-traffic-lights"]')).not.toBeNull()

    fireEvent.mouseDown(header, { button: 0, detail: 2, clientX: 200, clientY: 10 })
    expect(perform).not.toHaveBeenCalled()
    fireEvent.mouseUp(header, { button: 0, detail: 2, clientX: 220, clientY: 10 })
    expect(perform).not.toHaveBeenCalled()

    fireEvent.mouseDown(header, { button: 0, detail: 2, clientX: 200, clientY: 10 })
    fireEvent.mouseUp(header, { button: 0, detail: 2, clientX: 200, clientY: 10 })
    expect(perform).toHaveBeenCalledWith('titleBarDoubleClick')

    emit({ ...normal, fullscreen: true })
    expect(header.querySelector('[data-slot="title-bar-traffic-lights"]')).toBeNull()
  })

  it('places GTK controls on the side the decoration layout asks for', async () => {
    const { host } = fakeHost({ platform: 'linux', leading: ['close'], trailing: [] })
    const header = await renderTitleBar(host)

    const [first] = header.children
    expect(first).toHaveAttribute('data-slot', 'window-controls')
    expect(first.querySelector('[data-window-button="close"]')).not.toBeNull()
  })

  it('renders a plain bar without a window host', () => {
    render(<TitleBar host={null} title="MailX" />)
    expect(screen.getByRole('banner').querySelector('[data-window-button]')).toBeNull()
  })
})

describe('TitleBarActions', () => {
  it('places page actions beside the window controls without making them drag handles', async () => {
    const { host, perform } = fakeHost({ platform: 'windows', trailing: ['close'] })
    render(
      <TitleBarSlotsProvider>
        <TitleBar host={host} title="MailX" />
        <TitleBarActions side="leading"><button type="button">折叠</button></TitleBarActions>
        <TitleBarActions side="center"><input aria-label="搜索" /></TitleBarActions>
        <TitleBarActions side="trailing"><button type="button">更多</button></TitleBarActions>
      </TitleBarSlotsProvider>,
    )
    const header = screen.getByRole('banner')
    await vi.waitFor(() => expect(header.dataset.platform).toBe('windows'))
    const leading = header.querySelector('[data-slot="title-bar-leading"]')!
    const trailing = header.querySelector('[data-slot="title-bar-trailing"]')!
    expect(leading).toContainElement(await screen.findByRole('button', { name: '折叠' }))
    expect(trailing).toContainElement(screen.getByRole('button', { name: '更多' }))
    expect(header.querySelector('[data-slot="title-bar-center"]')).toContainElement(screen.getByRole('textbox', { name: '搜索' }))
    expect(trailing.nextElementSibling).toHaveAttribute('data-app-menu-trigger')
    expect(trailing.nextElementSibling?.nextElementSibling).toHaveAttribute('data-slot', 'window-controls')

    fireEvent.mouseDown(screen.getByRole('button', { name: '折叠' }), { button: 0, detail: 1 })
    expect(perform).not.toHaveBeenCalled()
  })
})

describe('previewPlatformFromUrl', () => {
  it('accepts only known platforms', () => {
    expect(previewPlatformFromUrl('?titlebar=linux')).toBe('linux')
    expect(previewPlatformFromUrl('?composer=x&platform=windows')).toBe('windows')
    expect(previewPlatformFromUrl('?platform=beos')).toBeNull()
    expect(previewPlatformFromUrl('?titlebar=beos')).toBeNull()
    expect(previewPlatformFromUrl('')).toBeNull()
  })
})
