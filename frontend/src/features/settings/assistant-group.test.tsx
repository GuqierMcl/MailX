import { describe, expect, it } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor } from '@/test/render'
import { exampleAccount, testClient } from '@/features/accounts/test-fixtures'
import type { AiConsent, AiModel, AiState, AssistantSettings } from '@/generated/accounts'
import { AiPane } from './ai-pane'
import { DEFAULT_ASSISTANT } from './settings-queries'

const MODELS: AiModel[] = [{ id: 'big', name: 'Big Model', contextLength: 200000, outputLength: 8000, toolCall: true, source: 'provider' }]
const CONNECTION = { id: 'c1', providerId: 'openai', name: 'OpenAI', protocol: 'openaiCompatible' as const, baseUrl: 'https://api.openai.com/v1', useProxy: false, hasKey: true }
const CONSENT: AiConsent = { connectionId: 'c1', baseUrl: 'https://api.openai.com/v1' }
const DEFAULT_MODEL = { connectionId: 'c1', modelId: 'big', name: 'Big Model', contextLength: null, outputLength: null }

/** The settings pane as the host serves it: saved changes come back, and consent changes bump the revision. */
async function setup({ assistant = {}, consent = null, chat = true }: { assistant?: Partial<AssistantSettings>; consent?: AiConsent | null; chat?: boolean } = {}) {
  const work = exampleAccount({ displayName: '工作邮箱', emailAddress: 'work@example.com' })
  const home = exampleAccount({ id: 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', displayName: '个人邮箱', emailAddress: 'home@example.com' })
  const client = testClient([work, home])
  const ai: AiState = { connections: [CONNECTION], defaultModel: DEFAULT_MODEL, translationModel: null, route: { kind: 'direct' }, proxyPasswordSet: false }
  client.aiState.mockResolvedValue(ai)
  client.aiCatalog.mockResolvedValue({ providers: [], updatedAt: null, stale: false, refreshError: null })
  client.aiListModels.mockResolvedValue(MODELS)
  const read = client.getSettings.getMockImplementation()!
  let current = consent
  let bumps = 0
  client.getSettings.mockImplementation(async () => {
    const snapshot = await read()
    return { ...snapshot, revision: snapshot.revision + bumps, settings: { ...snapshot.settings, assistant: { ...snapshot.settings.assistant, consented: current } } }
  })
  client.assistantRevokeConsent.mockImplementation(async () => { current = null; bumps += 1 })
  // Seed the client's own store, then forget the seeding write.
  await client.updateSettings({ assistant: { ...DEFAULT_ASSISTANT, ...assistant } })
  client.updateSettings.mockClear()
  render(<AiPane client={client} assistantChat={chat} />)
  return { client, work, home }
}

const lastAssistantPatch = (client: Awaited<ReturnType<typeof setup>>['client']) => {
  const calls = client.updateSettings.mock.calls
  return calls[calls.length - 1][0].assistant
}

describe('AI settings: assistant', () => {
  it('shows the group after the model and connection groups', async () => {
    await setup()
    await screen.findByRole('switch', { name: '启用助手' })
    const headings = [...document.querySelectorAll('legend')].map((legend) => legend.textContent)
    expect(headings).toEqual(['服务商', '默认模型', '助手'])
  })

  it('leaves out the rows only the chat uses when the pane is not given the chat', async () => {
    await setup({ chat: false })
    await screen.findByRole('switch', { name: '启用助手' })
    expect(screen.getByRole('combobox', { name: '对话模型' })).toBeInTheDocument()
    expect(await screen.findByRole('checkbox', { name: '工作邮箱' })).toBeInTheDocument()
    for (const name of ['整理邮件', 'AI 创建的草稿', '发送邮件']) expect(screen.queryByRole('combobox', { name })).not.toBeInTheDocument()
    expect(screen.queryByRole('textbox', { name: '自定义指令' })).not.toBeInTheDocument()
  })

  it('turns the assistant on and off, and disables everything else while off', async () => {
    const { client } = await setup()
    const enabled = await screen.findByRole('switch', { name: '启用助手' })
    expect(enabled).toBeChecked()
    expect(screen.getByRole('combobox', { name: '对话模型' })).toBeEnabled()
    await userEvent.click(enabled)
    await waitFor(() => expect(lastAssistantPatch(client)).toMatchObject({ enabled: false }))
    await waitFor(() => expect(screen.getByRole('combobox', { name: '对话模型' })).toBeDisabled())
    expect(screen.getByRole('combobox', { name: '整理邮件' })).toBeDisabled()
    expect(screen.getByRole('combobox', { name: 'AI 创建的草稿' })).toBeDisabled()
    expect(screen.getByRole('combobox', { name: '发送邮件' })).toBeDisabled()
    expect(screen.getByRole('checkbox', { name: '工作邮箱' })).toHaveAttribute('aria-disabled', 'true')
    expect(screen.getByRole('textbox', { name: '自定义指令' })).toBeDisabled()
    expect(screen.getByRole('switch', { name: '启用助手' })).toBeEnabled()
  })

  it('follows the default model, or saves a model from a connection', async () => {
    const { client } = await setup()
    const picker = await screen.findByRole('combobox', { name: '对话模型' })
    expect(picker).toHaveTextContent('跟随默认模型（Big Model）')
    await userEvent.click(picker)
    await userEvent.click(await screen.findByText('Big Model', { selector: '[cmdk-item] span, [role="option"] span' }))
    await waitFor(() => expect(lastAssistantPatch(client)).toMatchObject({ model: expect.objectContaining({ connectionId: 'c1', modelId: 'big' }) }))
    await userEvent.click(screen.getByRole('combobox', { name: '对话模型' }))
    await userEvent.click(await screen.findByText('跟随默认模型（Big Model）', { selector: '[cmdk-item]' }))
    await waitFor(() => expect(lastAssistantPatch(client)).toMatchObject({ model: null }))
  })

  it.each([
    ['整理邮件', '每次询问', { organize: 'ask' }],
    ['AI 创建的草稿', '每次询问', { aiDrafts: 'ask' }],
    ['发送邮件', '直接发送', { send: 'direct' }],
  ])('saves the %s choice', async (label, option, patch) => {
    const { client } = await setup()
    await userEvent.click(await screen.findByRole('combobox', { name: label }))
    await userEvent.click(await screen.findByRole('option', { name: option }))
    await waitFor(() => expect(lastAssistantPatch(client)).toMatchObject(patch))
  })

  it('warns under "直接发送" only while it is chosen', async () => {
    await setup({ assistant: {} })
    expect(await screen.findByRole('combobox', { name: '发送邮件' })).toHaveTextContent('每次确认')
    expect(screen.queryByText('AI 可以不经确认发出邮件。')).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole('combobox', { name: '发送邮件' }))
    await userEvent.click(await screen.findByRole('option', { name: '直接发送' }))
    expect(await screen.findByText('AI 可以不经确认发出邮件。')).toBeInTheDocument()
  })

  it('lists every mailbox and saves unticked ones as excluded', async () => {
    const { client, home } = await setup()
    const work = await screen.findByRole('checkbox', { name: '工作邮箱' })
    expect(work).toBeChecked()
    expect(screen.getByRole('checkbox', { name: '个人邮箱' })).toBeChecked()
    await userEvent.click(screen.getByRole('checkbox', { name: '个人邮箱' }))
    await waitFor(() => expect(lastAssistantPatch(client)).toMatchObject({ excludedAccounts: [home.id] }))
    await waitFor(() => expect(screen.getByRole('checkbox', { name: '个人邮箱' })).not.toBeChecked())
    await userEvent.click(screen.getByRole('checkbox', { name: '个人邮箱' }))
    await waitFor(() => expect(lastAssistantPatch(client)).toMatchObject({ excludedAccounts: [] }))
  })

  it('says consent is missing without a button, and shows the agreed service with a way to revoke it', async () => {
    const { client } = await setup()
    expect(await screen.findByText('尚未同意')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '撤销' })).not.toBeInTheDocument()
    expect(client.assistantGrantConsent).not.toHaveBeenCalled()
  })

  it('revokes the consent through the assistant command, never a settings write', async () => {
    const { client } = await setup({ consent: CONSENT })
    expect(await screen.findByText('已同意发送给OpenAI')).toBeInTheDocument()
    client.updateSettings.mockClear()
    await userEvent.click(screen.getByRole('button', { name: '撤销' }))
    expect(client.assistantRevokeConsent).toHaveBeenCalledTimes(1)
    expect(client.updateSettings).not.toHaveBeenCalled()
    expect(await screen.findByText('尚未同意')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '撤销' })).not.toBeInTheDocument()
  })

  it('saves the instructions when the field loses focus, and counts only near the limit', async () => {
    const { client } = await setup()
    const field = await screen.findByRole('textbox', { name: '自定义指令' })
    await userEvent.click(field)
    await userEvent.type(field, '叫我老师')
    expect(client.updateSettings).not.toHaveBeenCalled()
    expect(screen.queryByText(/\/ 2000/)).not.toBeInTheDocument()
    await userEvent.tab()
    await waitFor(() => expect(lastAssistantPatch(client)).toMatchObject({ instructions: '叫我老师' }))
    expect(field).toHaveAttribute('maxlength', '2000')
  })

  it('shows the counter within 200 characters of the limit', async () => {
    await setup({ assistant: { instructions: 'x'.repeat(1850) } })
    expect(await screen.findByText('1850 / 2000')).toBeInTheDocument()
  })
})
