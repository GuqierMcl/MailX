import { act, fireEvent, render, screen } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'
import { i18n } from '@/i18n'
import { TitleBar, TitleBarActions, TitleBarSlotsProvider } from './title-bar'
import { previewPlatformFromUrl, type WindowChrome, type WindowHost, type WindowState } from './window-host'

const normal: WindowState = { maximized: false, fullscreen: false, focused: true }

function fakeHost(chrome: Partial<WindowChrome> & Pick<WindowChrome, 'platform'>) {
  let emit: (state: WindowState) => void = () => undefined
  const host: WindowHost = {
    getChrome: async () => ({ leading: [], trailing: [], state: normal, ...chrome }),
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
    // The trailing actions sit directly before the window controls.
    expect(trailing.nextElementSibling).toHaveAttribute('data-slot', 'window-controls')

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
