import { act, fireEvent, render, screen, waitFor, within } from '@/test/render'
import { invokeAppMenuCommand } from '@/features/app-menu/commands'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { createMockClient } from '@/lib/mock-client'
import type { MailxClient } from '@/lib/mailx-client'
import { withTitleBar } from '@/test/title-bar'
import { Workbench } from './workbench'

afterEach(() => localStorage.clear())

/** A mock client with a model to talk to and the consent given, as a configured assistant has. */
async function withAssistant(client: MailxClient = createMockClient()) {
  const { connectionId } = await client.aiSaveConnection({ id: null, providerId: 'custom', name: 'Mock', protocol: 'openaiCompatible', baseUrl: 'https://example.test', useProxy: false, apiKey: 'k', verify: false })
  await client.aiSetDefaultModel({ connectionId, modelId: 'large-1', name: 'Large 1', contextLength: 200000, outputLength: 32000, toolCall: true })
  await client.assistantGrantConsent()
  return client
}

const capsule = () => screen.findByRole('button', { name: /^问问/ })
const island = () => screen.findByRole('complementary', { name: 'AI 助手' })
const list = () => document.querySelector('[data-slot="workbench-list"]')!

async function openFirstEmail() {
  const rows = await screen.findByRole('list', { name: '邮件列表' })
  await waitFor(() => expect(rows.querySelector('[data-email-id]')).not.toBeNull())
  await userEvent.click(rows.querySelector<HTMLElement>('[data-email-id]')!)
  await screen.findByRole('toolbar', { name: '邮件操作' })
}

describe('the assistant beside the mail', () => {
  it('uses the assistant shortcut from its input after menu opening and respects IME and handled keys', async () => {
    render(withTitleBar(<Workbench client={await withAssistant()} />))
    await capsule()
    act(() => { expect(invokeAppMenuCommand('toggleAssistant')).toBe(true) })
    const input = within(await island()).getByRole('textbox')
    await waitFor(() => expect(input).toHaveFocus())
    const shortcut = { code: 'KeyJ', key: 'j', metaKey: true }
    fireEvent.keyDown(input, { ...shortcut, isComposing: true })
    expect(input).toBeInTheDocument()
    input.addEventListener('keydown', (event) => event.preventDefault(), { once: true })
    fireEvent.keyDown(input, shortcut)
    expect(input).toBeInTheDocument()
    fireEvent.keyDown(input, shortcut)
    expect(input).not.toBeInTheDocument()
  })

  it('has no capsule without a usable model', async () => {
    render(withTitleBar(<Workbench client={createMockClient()} />))
    await screen.findByRole('list', { name: '邮件列表' })
    expect(screen.queryByRole('button', { name: /^问问/ })).not.toBeInTheDocument()
  })

  it('opens beside the reader, shows one pane at a time and leaves the saved layout alone', async () => {
    const client = await withAssistant()
    render(withTitleBar(<Workbench client={client} />))
    expect(await capsule()).toHaveTextContent('问问你的邮件')
    await openFirstEmail()
    await userEvent.click(await capsule())

    const aside = await island()
    // The reader stays in view and the list steps aside, as in single-pane reading.
    await waitFor(() => expect(list()).toHaveAttribute('aria-hidden', 'true'))
    const reader = document.querySelector<HTMLElement>('[data-slot="workbench-reader"]')!
    const subject = within(reader).getByRole('heading', { level: 2 }).textContent!
    expect(within(aside).getByText(subject)).toBeInTheDocument()
    await waitFor(() => expect(within(aside).getByRole('textbox')).toHaveFocus())
    expect(screen.queryByRole('button', { name: /^问问/ })).not.toBeInTheDocument()

    await userEvent.click(within(aside).getByRole('button', { name: '关闭 AI 助手' }))
    await waitFor(() => expect(screen.queryByRole('complementary', { name: 'AI 助手' })).not.toBeInTheDocument())
    expect(list()).not.toHaveAttribute('aria-hidden')
    expect((await client.getSettings()).settings.layout.readingMode).toBe('split')
  })

  it('sends what is on screen with the message, unless that is dropped', async () => {
    const client = await withAssistant()
    const start = vi.spyOn(client, 'assistantStart')
    render(withTitleBar(<Workbench client={client} />))
    await openFirstEmail()
    await userEvent.click(await capsule())
    const aside = await island()
    const field = within(aside).getByRole('textbox')
    await userEvent.type(field, '这封说了什么{Enter}')
    await waitFor(() => expect(start).toHaveBeenCalled())
    const first = start.mock.calls.at(-1)![0].kind
    expect(first).toMatchObject({ type: 'chat', message: '这封说了什么', context: { type: 'email' } })
    await waitFor(() => expect(within(aside).getByRole('textbox')).not.toHaveAttribute('readonly'), { timeout: 8000 })

    await userEvent.click(within(aside).getByRole('button', { name: '不带上这项' }))
    await userEvent.type(within(aside).getByRole('textbox'), '还有别的吗{Enter}')
    await waitFor(() => expect(start.mock.calls.at(-1)![0].kind).toMatchObject({ message: '还有别的吗' }))
    expect(start.mock.calls.at(-1)![0].kind).not.toHaveProperty('context')
  }, 15000)

  it('toggles with Cmd/Ctrl+J and hands the conversation to the Now page', async () => {
    const client = await withAssistant()
    render(withTitleBar(<Workbench client={client} />))
    await capsule()
    act(() => { window.dispatchEvent(new KeyboardEvent('keydown', { key: 'j', code: 'KeyJ', ctrlKey: true, bubbles: true })) })
    const aside = await island()
    await userEvent.type(within(aside).getByRole('textbox'), '你好{Enter}')
    await within(aside).findByText('你好')

    const nav = within(screen.getByRole('navigation', { name: '邮箱导航' }))
    await userEvent.click(nav.getByRole('button', { name: '此刻' }))
    await waitFor(() => expect(screen.queryByRole('complementary', { name: 'AI 助手' })).not.toBeInTheDocument())
    // The Now page opens on the same conversation, not on its overview.
    expect(await screen.findByRole('button', { name: '返回' })).toBeInTheDocument()
    expect(screen.getByRole('log')).toHaveTextContent('你好')

    await userEvent.click(nav.getByRole('button', { name: '收件箱' }))
    expect(await capsule()).toBeInTheDocument()
    act(() => { window.dispatchEvent(new KeyboardEvent('keydown', { key: 'j', code: 'KeyJ', metaKey: true, bubbles: true })) })
    await island()
    act(() => { window.dispatchEvent(new KeyboardEvent('keydown', { key: 'j', code: 'KeyJ', metaKey: true, bubbles: true })) })
    await waitFor(() => expect(screen.queryByRole('complementary', { name: 'AI 助手' })).not.toBeInTheDocument())
  }, 15000)

  it('keeps single keys and Esc inside the island from acting on the mail', async () => {
    const client = await withAssistant()
    render(withTitleBar(<Workbench client={client} />))
    await openFirstEmail()
    await userEvent.click(await capsule())
    const aside = await island()
    within(aside).getByRole('button', { name: '新对话' }).focus()
    await userEvent.keyboard('e{Escape}')
    // Still open: neither archived (e) nor closed (Esc).
    expect(screen.getByRole('toolbar', { name: '邮件操作' })).toBeInTheDocument()
    expect(screen.getByRole('complementary', { name: 'AI 助手' })).toBeInTheDocument()
  })
})
