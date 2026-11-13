import { render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { testClient } from '@/features/accounts/test-fixtures'
import { SHORTCUTS } from '@/features/mail/shortcuts'
import { ShortcutsPane } from './shortcuts-pane'

describe('keyboard shortcut settings', () => {
  it('lists every shortcut by group and turns the single keys off and on', async () => {
    const client = testClient()
    const user = userEvent.setup()
    const { container } = render(<ShortcutsPane client={client} />)
    const toggle = await screen.findByRole('switch', { name: '单键快捷键' })
    await waitFor(() => expect(toggle).not.toBeDisabled())
    expect(toggle).toBeChecked()
    expect(screen.getByText(/E 归档、S 加星标/)).toBeInTheDocument()
    for (const name of ['浏览', '处理', '回复']) expect(screen.getByRole('heading', { name })).toBeInTheDocument()
    const listed = [...container.querySelectorAll('[data-shortcut]')].map((row) => row.getAttribute('data-shortcut'))
    expect(listed).toEqual(SHORTCUTS.map((item) => item.id))
    expect(container.querySelector('[data-off]')).toBeNull()

    await user.click(toggle)
    await waitFor(() => expect(client.updateSettings).toHaveBeenLastCalledWith({ general: { language: 'zh-CN', autoUpdate: true, closeBehavior: 'ask', showMenuBarIcon: true, silentStart: false, singleKeyShortcuts: false } }))
    await waitFor(() => expect(toggle).not.toBeChecked())
    expect(screen.getByText('单键快捷键已关闭，带修饰键的快捷键和 Esc 仍可用。')).toBeInTheDocument()
    const off = [...container.querySelectorAll('[data-off]')].map((row) => row.getAttribute('data-shortcut'))
    expect(off).toEqual(SHORTCUTS.filter((item) => item.singleKey).map((item) => item.id))
    // Modifier shortcuts stay as they are.
    const compose = container.querySelector<HTMLElement>('[data-shortcut="compose"]')!
    expect(compose).not.toHaveAttribute('data-off')
    expect(within(compose).getByText('N')).toBeInTheDocument()
    expect(within(container.querySelector<HTMLElement>('[data-shortcut="archive"]')!).getByText('（已关闭）')).toBeInTheDocument()

    await waitFor(() => expect(toggle).not.toBeDisabled())
    await user.click(toggle)
    await waitFor(() => expect(client.updateSettings).toHaveBeenLastCalledWith({ general: expect.objectContaining({ singleKeyShortcuts: true }) }))
    await waitFor(() => expect(container.querySelector('[data-off]')).toBeNull())
  })
})
