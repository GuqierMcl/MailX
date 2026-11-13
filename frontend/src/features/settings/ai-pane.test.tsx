import { describe, expect, it } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor, within } from '@/test/render'
import { testClient } from '@/features/accounts/test-fixtures'
import type { AiCatalog, AiConnectionState, AiModel, AiState, SaveAiConnection } from '@/generated/accounts'
import { MailxClientError } from '@/lib/mailx-client'
import { AiPane } from './ai-pane'
import { AiProviderIcon } from './ai-provider-icon'
import { SettingsDialog } from './settings-dialog'

const MODELS: AiModel[] = [
  { id: 'big', name: 'Big Model', contextLength: 200000, outputLength: 8000, toolCall: true, source: 'provider' },
  { id: 'small', name: 'Small Model', contextLength: null, outputLength: null, toolCall: null, source: 'provider' },
]
const CATALOG: AiCatalog = {
  providers: [
    { id: 'openai', name: 'OpenAI', protocol: 'openaiCompatible', baseUrl: 'https://api.openai.com/v1', builtIn: true },
    { id: 'ollama', name: 'Ollama', protocol: 'openaiCompatible', baseUrl: 'http://localhost:11434/v1', builtIn: true, local: true },
    { id: 'zeta', name: 'Zeta', protocol: 'openaiCompatible', baseUrl: 'https://zeta.example/v1', builtIn: false },
    { id: 'alpha', name: 'Alpha', protocol: 'anthropic', baseUrl: 'https://alpha.example/v1', builtIn: false },
  ],
  updatedAt: null, stale: false, refreshError: null,
}
const failure = (code: 'aiKeyRejected' | 'aiModelNotFound' | 'aiNetwork') => new MailxClientError({ code, message: 'x', field: null })
const connection = (over: Partial<AiConnectionState> = {}): AiConnectionState => ({
  id: 'c1', providerId: 'openai', name: 'OpenAI', protocol: 'openaiCompatible', baseUrl: 'https://api.openai.com/v1', useProxy: false, hasKey: true, ...over,
})

/** A client whose AI settings behave like the real ones: saved changes come back in the next state. */
function setup(initial: Partial<AiState> = {}, catalog: AiCatalog = CATALOG) {
  const client = testClient()
  let state: AiState = { connections: [], defaultModel: null, translationModel: null, route: { kind: 'direct' }, proxyPasswordSet: false, ...initial }
  client.aiState.mockImplementation(async () => state)
  client.aiCatalog.mockResolvedValue(catalog)
  client.aiListModels.mockResolvedValue(MODELS)
  client.aiSaveConnection.mockImplementation(async (request: SaveAiConnection) => {
    const id = request.id ?? 'new-1'
    const old = state.connections.find((entry) => entry.id === id)
    const saved: AiConnectionState = { id, providerId: request.providerId, name: request.name, protocol: request.protocol, baseUrl: request.baseUrl, useProxy: request.useProxy,
      hasKey: request.apiKey === null ? old?.hasKey ?? false : request.apiKey !== '' }
    state = { ...state, connections: old ? state.connections.map((entry) => entry.id === id ? saved : entry) : [...state.connections, saved] }
    return { state, connectionId: id, models: request.verify ? MODELS : null }
  })
  client.aiRemoveConnection.mockImplementation(async (id: string) => {
    state = { ...state, connections: state.connections.filter((entry) => entry.id !== id), defaultModel: state.defaultModel?.connectionId === id ? null : state.defaultModel, translationModel: state.translationModel?.connectionId === id ? null : state.translationModel }
    return state
  })
  client.aiSetDefaultModel.mockImplementation(async (model) => { state = { ...state, defaultModel: model }; return state })
  render(<AiPane client={client} />)
  return client
}

describe('AI settings: providers', () => {
  it('starts empty with one line and an add button', async () => {
    setup()
    expect(await screen.findByText('还没有连接服务商。')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '添加服务商' })).toBeInTheDocument()
    expect(screen.getByRole('combobox', { name: '默认模型' })).toBeDisabled()
    expect(screen.getByText('先添加服务商。')).toBeInTheDocument()
  })

  it('adds a built-in provider, connects, and saves the chosen model as the default', async () => {
    const client = setup()
    await userEvent.click(await screen.findByRole('button', { name: '添加服务商' }))
    const dialog = await screen.findByRole('dialog')
    // Built-ins first, then Custom, then the rest by name.
    const rows = within(dialog).getAllByRole('option').map((row) => row.textContent)
    expect(rows).toEqual(['O', 'Ollama本机', '自定义', 'Alpha', 'Zeta'].map((name, index) => index === 0 ? expect.stringContaining('OpenAI') : name === 'Ollama本机' ? expect.stringContaining('Ollama') : expect.stringContaining(name)))
    expect(within(dialog).getByText('本机')).toBeInTheDocument()
    await userEvent.click(within(dialog).getByRole('option', { name: /OpenAI/ }))
    expect(await within(dialog).findByLabelText('名称')).toHaveValue('OpenAI')
    expect(within(dialog).getByLabelText('接口地址')).toHaveValue('https://api.openai.com/v1')
    expect(within(dialog).queryByLabelText('协议')).not.toBeInTheDocument()
    expect(within(dialog).queryByLabelText('使用代理')).not.toBeInTheDocument()
    await userEvent.type(within(dialog).getByLabelText('API 密钥'), ' sk-test ')
    await userEvent.click(within(dialog).getByRole('button', { name: '连接' }))
    expect(client.aiSaveConnection).toHaveBeenCalledWith({
      id: null, providerId: 'openai', name: 'OpenAI', protocol: 'openaiCompatible', baseUrl: 'https://api.openai.com/v1', useProxy: false, apiKey: 'sk-test', verify: true,
    })
    expect(await screen.findByRole('heading', { name: '选择模型' })).toBeInTheDocument()
    await userEvent.click(await screen.findByRole('option', { name: /Big Model/ }))
    await waitFor(() => expect(client.aiSetDefaultModel).toHaveBeenCalledWith({ connectionId: 'new-1', modelId: 'big', name: 'Big Model', contextLength: 200000, outputLength: 8000 }))
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
    expect(screen.getByRole('combobox', { name: '默认模型' })).toHaveTextContent('Big Model')
    expect(screen.getByText('api.openai.com')).toBeInTheDocument()
  })

  it('refreshes a stale catalog in the background and ignores a failed refresh', async () => {
    const client = setup({}, { ...CATALOG, stale: true })
    client.aiCatalog.mockImplementation(async (refresh) => {
      if (refresh) throw failure('aiNetwork')
      return { ...CATALOG, stale: true }
    })
    await userEvent.click(await screen.findByRole('button', { name: '添加服务商' }))
    expect(await screen.findByRole('option', { name: /Zeta/ })).toBeInTheDocument()
    await waitFor(() => expect(client.aiCatalog).toHaveBeenCalledWith(true))
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
  })

  it('shows the protocol only for a custom provider and the proxy switch only when a proxy is set', async () => {
    const client = setup({ route: { kind: 'proxy', proxyKind: 'socks5', host: 'p', port: 1080, username: '' } })
    await userEvent.click(await screen.findByRole('button', { name: '添加服务商' }))
    await userEvent.click(await screen.findByRole('option', { name: /自定义/ }))
    const dialog = await screen.findByRole('dialog')
    expect(within(dialog).getByLabelText('名称')).toHaveValue('')
    expect(within(dialog).getByLabelText('协议')).toHaveTextContent('OpenAI 兼容')
    expect(within(dialog).getByRole('switch', { name: '使用代理' })).toBeChecked()
    expect(within(dialog).getByRole('button', { name: '连接' })).toBeDisabled()
    await userEvent.type(within(dialog).getByLabelText('名称'), 'Mine')
    await userEvent.type(within(dialog).getByLabelText('接口地址'), 'http://localhost:8080/v1')
    await userEvent.click(within(dialog).getByRole('button', { name: '连接' }))
    expect(client.aiSaveConnection).toHaveBeenCalledWith(expect.objectContaining({ providerId: 'custom', name: 'Mine', useProxy: true, apiKey: null, verify: true }))
  })

  it('shows the error when connecting fails, and can save anyway', async () => {
    const client = setup()
    const save = client.aiSaveConnection.getMockImplementation()!
    client.aiSaveConnection.mockImplementationOnce(async () => { throw failure('aiKeyRejected') })
    await userEvent.click(await screen.findByRole('button', { name: '添加服务商' }))
    await userEvent.click(await screen.findByRole('option', { name: /OpenAI/ }))
    await userEvent.type(await screen.findByLabelText('API 密钥'), 'bad')
    await userEvent.click(screen.getByRole('button', { name: '连接' }))
    expect(await screen.findByText('密钥无效或没有权限')).toBeInTheDocument()
    expect(client.aiSetDefaultModel).not.toHaveBeenCalled()
    client.aiSaveConnection.mockImplementation(save)
    await userEvent.click(screen.getByRole('button', { name: '仍然保存' }))
    expect(client.aiSaveConnection).toHaveBeenLastCalledWith(expect.objectContaining({ apiKey: 'bad', verify: false }))
    // The models were not fetched while saving, so the next step lists them itself.
    expect(await screen.findByRole('heading', { name: '选择模型' })).toBeInTheDocument()
    expect(await screen.findByRole('option', { name: /Small Model/ })).toBeInTheDocument()
    expect(client.aiListModels).toHaveBeenCalledWith('new-1')
  })

  it('closes after saving when a default model already exists', async () => {
    const model = { connectionId: 'c1', modelId: 'big', name: 'Big Model', contextLength: null, outputLength: null }
    const client = setup({ connections: [connection()], defaultModel: model })
    await userEvent.click(await screen.findByRole('button', { name: '添加服务商' }))
    await userEvent.click(await screen.findByRole('option', { name: /Alpha/ }))
    await userEvent.click(await screen.findByRole('button', { name: '连接' }))
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
    expect(client.aiSetDefaultModel).not.toHaveBeenCalled()
  })

  it('edits without touching the key unless asked, and clears it on request', async () => {
    const client = setup({ connections: [connection()] })
    await userEvent.click(await screen.findByRole('button', { name: '编辑 OpenAI' }))
    const dialog = await screen.findByRole('dialog')
    expect(within(dialog).getByLabelText('API 密钥')).toHaveAttribute('placeholder', '已保存')
    await userEvent.clear(within(dialog).getByLabelText('名称'))
    await userEvent.type(within(dialog).getByLabelText('名称'), 'Work')
    await userEvent.click(within(dialog).getByRole('button', { name: '连接' }))
    expect(client.aiSaveConnection).toHaveBeenLastCalledWith(expect.objectContaining({ id: 'c1', name: 'Work', apiKey: null }))
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
    expect(screen.getByText('Work')).toBeInTheDocument()

    await userEvent.click(screen.getByRole('button', { name: '编辑 Work' }))
    await userEvent.click(await screen.findByRole('button', { name: '清除' }))
    await userEvent.click(screen.getByRole('button', { name: '连接' }))
    expect(client.aiSaveConnection).toHaveBeenLastCalledWith(expect.objectContaining({ id: 'c1', apiKey: '' }))
    expect(await screen.findByText('未设置密钥')).toBeInTheDocument()
  })

  it('marks a provider without a key, except one that runs on this computer', async () => {
    setup({ connections: [
      connection({ id: 'a', name: 'No key', hasKey: false }),
      connection({ id: 'b', providerId: 'ollama', name: 'Home', hasKey: false, baseUrl: 'http://localhost:11434/v1' }),
    ] })
    expect(await screen.findByText('localhost:11434')).toBeInTheDocument()
    await waitFor(() => expect(screen.getAllByText('未设置密钥')).toHaveLength(1))
  })

  it('removes a provider after one confirmation and clears the default model that used it', async () => {
    const model = { connectionId: 'c1', modelId: 'big', name: 'Big Model', contextLength: null, outputLength: null }
    const client = setup({ connections: [connection()], defaultModel: model })
    await userEvent.click(await screen.findByRole('button', { name: '移除 OpenAI' }))
    expect(client.aiRemoveConnection).not.toHaveBeenCalled()
    await userEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: '移除' }))
    expect(client.aiRemoveConnection).toHaveBeenCalledWith('c1')
    expect(await screen.findByText('还没有连接服务商。')).toBeInTheDocument()
    expect(screen.getByRole('combobox', { name: '默认模型' })).toBeDisabled()
  })
})

describe('AI settings: default model', () => {
  const two = [connection(), connection({ id: 'c2', name: 'Second', providerId: 'anthropic' })]

  it('lists each connection live when opened, keeps going when one fails, and saves a snapshot', async () => {
    const client = setup({ connections: two })
    client.aiListModels.mockImplementation(async (id) => { if (id === 'c2') throw failure('aiNetwork'); return MODELS })
    await userEvent.click(await screen.findByRole('combobox', { name: '默认模型' }))
    expect(client.aiListModels).toHaveBeenCalledTimes(2)
    expect(await screen.findByText('连不上服务商')).toBeInTheDocument()
    expect(screen.getByText('200K 上下文')).toBeInTheDocument()
    await userEvent.click(screen.getByRole('option', { name: /Small Model/ }))
    await waitFor(() => expect(client.aiSetDefaultModel).toHaveBeenCalledWith({ connectionId: 'c1', modelId: 'small', name: 'Small Model', contextLength: null, outputLength: null }))
    expect(await screen.findByRole('combobox', { name: '默认模型' })).toHaveTextContent('Small Model')
  })

  it('searches, and accepts a typed model id for the chosen provider', async () => {
    const client = setup({ connections: two })
    await userEvent.click(await screen.findByRole('combobox', { name: '默认模型' }))
    await userEvent.type(await screen.findByPlaceholderText('搜索或输入模型 ID'), 'big')
    await waitFor(() => expect(screen.queryAllByRole('option', { name: /Small Model/ })).toHaveLength(0))
    expect(screen.getAllByRole('option', { name: /Big Model/ })).toHaveLength(2)
    await userEvent.clear(screen.getByPlaceholderText('搜索或输入模型 ID'))
    await userEvent.type(screen.getByPlaceholderText('搜索或输入模型 ID'), 'my-model')
    const manual = await screen.findAllByRole('option', { name: '使用“my-model”' })
    expect(manual).toHaveLength(2)
    await userEvent.click(manual[1])
    await waitFor(() => expect(client.aiSetDefaultModel).toHaveBeenCalledWith({ connectionId: 'c2', modelId: 'my-model', name: 'my-model', contextLength: null, outputLength: null }))
  })

  it('tests the default model and shows how long it took', async () => {
    const model = { connectionId: 'c1', modelId: 'big', name: 'Big Model', contextLength: null, outputLength: null }
    const client = setup({ connections: [connection()], defaultModel: model })
    expect(await screen.findByRole('button', { name: '测试' })).toBeEnabled()
    client.aiTest.mockResolvedValueOnce({ latencyMs: 1234 })
    await userEvent.click(screen.getByRole('button', { name: '测试' }))
    expect(client.aiTest).toHaveBeenCalledWith('c1', 'big')
    expect(await screen.findByText('可用 · 1.2 秒')).toBeInTheDocument()
    client.aiTest.mockRejectedValueOnce(failure('aiModelNotFound'))
    await userEvent.click(screen.getByRole('button', { name: '测试' }))
    expect(await screen.findByText('找不到这个模型')).toBeInTheDocument()
    expect(screen.queryByText('可用 · 1.2 秒')).not.toBeInTheDocument()
  })

  it('keeps the proxy settings folded under Advanced', async () => {
    setup({ connections: [connection()] })
    expect(screen.queryByLabelText('连接方式')).not.toBeInTheDocument()
    await userEvent.click(await screen.findByRole('button', { name: /^高级/ }))
    expect(await screen.findByRole('combobox', { name: '连接方式' })).toHaveTextContent('直接连接')
  })
})

describe('provider icon', () => {
  const svg = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M0 0h24v24z"/></svg>'

  it('draws a downloaded logo as a mask and never puts the SVG in the page', async () => {
    const client = testClient()
    client.aiProviderIcon.mockResolvedValue({ mimeType: 'image/svg+xml', data: btoa(svg) })
    const { container } = render(<AiProviderIcon client={client} providerId="groq" name="Groq" />)
    const mask = await screen.findByTestId('ai-provider-mask')
    expect(mask.style.maskImage).toContain(`data:image/svg+xml;base64,${btoa(svg)}`)
    expect(container.querySelector('svg')).toBeNull()
    expect(container.innerHTML).not.toContain('<path')
  })

  it('uses the bundled logo of a built-in provider without asking the host', () => {
    const client = testClient()
    render(<AiProviderIcon client={client} providerId="openai" name="OpenAI" />)
    expect(screen.getByTestId('ai-provider-mask').style.maskImage).toMatch(/^url\("(data:image\/svg\+xml|\/).*svg/)
    expect(client.aiProviderIcon).not.toHaveBeenCalled()
  })

  it('falls back to the first letter on failure, when nothing is found, and for custom', async () => {
    const client = testClient()
    client.aiProviderIcon.mockRejectedValueOnce(failure('aiNetwork'))
    const { unmount } = render(<AiProviderIcon client={client} providerId="x" name="zeta" />)
    expect(await screen.findByTestId('ai-provider-initial')).toHaveTextContent('Z')
    await waitFor(() => expect(client.aiProviderIcon).toHaveBeenCalled())
    expect(screen.getByTestId('ai-provider-initial')).toBeInTheDocument()
    unmount()
    client.aiProviderIcon.mockResolvedValueOnce(null)
    const second = render(<AiProviderIcon client={client} providerId="y" name="Yo" />)
    await waitFor(() => expect(client.aiProviderIcon).toHaveBeenCalledTimes(2))
    expect(second.container.querySelector('[data-testid="ai-provider-initial"]')).toHaveTextContent('Y')
    second.unmount()
    render(<AiProviderIcon client={client} providerId="custom" name="Mine" />)
    expect(screen.getByTestId('ai-provider-initial')).toHaveTextContent('M')
    expect(client.aiProviderIcon).toHaveBeenCalledTimes(2)
  })

  it('refuses data that is not base64 SVG', async () => {
    const client = testClient()
    client.aiProviderIcon.mockResolvedValue({ mimeType: 'image/svg+xml', data: '");}</style><script>' })
    render(<AiProviderIcon client={client} providerId="evil" name="Evil" />)
    await waitFor(() => expect(client.aiProviderIcon).toHaveBeenCalled())
    expect(screen.getByTestId('ai-provider-initial')).toBeInTheDocument()
  })
})

describe('settings navigation', () => {
  it('lists AI in the integrations group, before MCP', async () => {
    render(<SettingsDialog client={testClient()} target={{ section: 'general' }} onClose={() => undefined} />)
    const group = await screen.findByTestId('settings-nav-group-integrations')
    expect(within(group).getAllByRole('button').map((button) => button.textContent)).toEqual(['AI', 'MCP'])
  })
})
