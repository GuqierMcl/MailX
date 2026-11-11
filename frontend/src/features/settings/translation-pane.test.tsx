import { describe, expect, it } from 'vitest'
import userEvent from '@testing-library/user-event'
import { act, render, screen, waitFor, within } from '@/test/render'
import { testClient } from '@/features/accounts/test-fixtures'
import type { TranslationKeyState, TranslationSettings } from '@/generated/accounts'
import { i18n } from '@/i18n'
import { MailxClientError } from '@/lib/mailx-client'
import { DEFAULT_TRANSLATION } from './settings-queries'
import { TranslationPane } from './translation-pane'

async function setup(translation: Partial<TranslationSettings> = {}, keys: TranslationKeyState[] = []) {
  const client = testClient()
  await client.updateSettings({ translation: { ...DEFAULT_TRANSLATION, ...translation } })
  client.updateSettings.mockClear()
  client.translationKeys.mockResolvedValue(keys)
  render(<TranslationPane client={client} />)
  const saved = () => client.updateSettings.mock.calls.at(-1)?.[0].translation
  return { client, saved }
}

const failure = (code: 'translationKeyRejected' | 'translationNetwork') => new MailxClientError({ code, message: 'x', field: null })
const choose = async (trigger: string | RegExp, option: string) => {
  await userEvent.click(await screen.findByRole('combobox', { name: trigger }))
  await userEvent.click(await screen.findByRole('option', { name: option }))
}

describe('translation settings: service and key', () => {
  it('chooses a service, and only then asks for its key', async () => {
    const { saved } = await setup()
    expect(await screen.findByRole('combobox', { name: '服务' })).toHaveTextContent('未设置')
    expect(screen.queryByLabelText('密钥')).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '测试' })).not.toBeInTheDocument()
    await choose('服务', 'DeepL')
    await waitFor(() => expect(saved()).toEqual(expect.objectContaining({ provider: 'deepl', consentedProvider: null })))
    expect(await screen.findByLabelText('密钥')).toBeInTheDocument()
  })

  it('keeps consent as it was when the service changes', async () => {
    const { saved } = await setup({ provider: 'deepl', consentedProvider: 'deepl' })
    await choose('服务', 'Google Cloud Translation')
    await waitFor(() => expect(saved()).toEqual(expect.objectContaining({ provider: 'google', consentedProvider: 'deepl' })))
  })

  it('saves, replaces and removes the key without ever showing it again', async () => {
    const { client } = await setup({ provider: 'deepl' })
    client.setTranslationKey.mockResolvedValue([{ provider: 'deepl', configured: true }])
    const input = await screen.findByLabelText('密钥')
    expect(input).toHaveAttribute('type', 'password')
    expect(input).toHaveAttribute('autocomplete', 'off')
    expect(input).toHaveAttribute('spellcheck', 'false')
    expect(screen.getByRole('button', { name: '测试' })).toBeDisabled()
    await userEvent.type(input, 'secret-key-1')
    await userEvent.click(screen.getByRole('button', { name: '保存' }))
    expect(client.setTranslationKey).toHaveBeenCalledExactlyOnceWith('deepl', 'secret-key-1')
    expect(await screen.findByText('已保存')).toBeInTheDocument()
    expect(screen.queryByLabelText('密钥')).not.toBeInTheDocument()
    expect(screen.queryByDisplayValue('secret-key-1')).not.toBeInTheDocument()
    expect(document.body.innerHTML).not.toContain('secret-key-1')
    expect(screen.getByRole('button', { name: '测试' })).toBeEnabled()

    await userEvent.click(screen.getByRole('button', { name: '更换' }))
    const again = await screen.findByLabelText('密钥')
    expect(again).toHaveValue('')
    await userEvent.type(again, 'secret-key-2')
    await userEvent.click(screen.getByRole('button', { name: '保存' }))
    expect(client.setTranslationKey).toHaveBeenLastCalledWith('deepl', 'secret-key-2')
    await screen.findByText('已保存')
    expect(document.body.innerHTML).not.toContain('secret-key-2')

    client.removeTranslationKey.mockResolvedValue([{ provider: 'deepl', configured: false }])
    await userEvent.click(screen.getByRole('button', { name: '移除' }))
    expect(client.removeTranslationKey).toHaveBeenCalledExactlyOnceWith('deepl')
    expect(await screen.findByLabelText('密钥')).toHaveValue('')
    expect(screen.getByRole('button', { name: '测试' })).toBeDisabled()
  })

  it('shows a failed save in place and keeps nothing in the field', async () => {
    const { client } = await setup({ provider: 'deepl' })
    client.setTranslationKey.mockRejectedValue(failure('translationKeyRejected'))
    await userEvent.type(await screen.findByLabelText('密钥'), 'wrong-key')
    await userEvent.click(screen.getByRole('button', { name: '保存' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('不接受这个密钥')
    expect(screen.getByLabelText('密钥')).toHaveValue('')
    expect(screen.queryByText('已保存')).not.toBeInTheDocument()
  })
})

describe('translation settings: region, test and network', () => {
  it('saves a valid region on leaving the field and flags an invalid one', async () => {
    const { client, saved } = await setup({ provider: 'microsoft' })
    const region = await screen.findByLabelText('区域')
    expect(region).toHaveAttribute('placeholder', 'eastasia')
    await userEvent.type(region, 'East Asia')
    await userEvent.tab()
    expect(await screen.findByRole('alert')).toHaveTextContent('小写字母、数字和连字符')
    expect(client.updateSettings).not.toHaveBeenCalled()
    await userEvent.clear(region)
    await userEvent.type(region, 'eastasia')
    await userEvent.tab()
    await waitFor(() => expect(saved()).toEqual(expect.objectContaining({ microsoftRegion: 'eastasia' })))
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
  })

  it('has no region for the other services', async () => {
    await setup({ provider: 'deepl' })
    await screen.findByLabelText('密钥')
    expect(screen.queryByLabelText('区域')).not.toBeInTheDocument()
  })

  it('tests the saved key and shows the sample or the reason', async () => {
    const { client } = await setup({ provider: 'deepl' }, [{ provider: 'deepl', configured: true }])
    client.testTranslation.mockRejectedValueOnce(failure('translationNetwork'))
    client.testTranslation.mockResolvedValueOnce({ translated: '你好，这是一条测试。' })
    await waitFor(() => expect(screen.getByRole('button', { name: '测试' })).toBeEnabled())
    await userEvent.click(screen.getByRole('button', { name: '测试' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('无法连接翻译服务')
    await userEvent.click(screen.getByRole('button', { name: '测试' }))
    expect(await screen.findByRole('status')).toHaveTextContent('可以使用：你好，这是一条测试。')
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
  })

  it('saves a proxy only once the address and port are valid, and direct at once', async () => {
    const { client, saved } = await setup()
    await choose('网络', '通过代理')
    expect(await screen.findByRole('alert')).toHaveTextContent('端口为 1 到 65535')
    expect(client.updateSettings).not.toHaveBeenCalled()
    const host = screen.getByLabelText('代理地址')
    const port = screen.getByLabelText('端口')
    await userEvent.type(host, 'proxy.example.com')
    await userEvent.type(port, '99999')
    await userEvent.tab()
    expect(client.updateSettings).not.toHaveBeenCalled()
    expect(screen.getByRole('alert')).toBeInTheDocument()
    await userEvent.clear(port)
    await userEvent.type(port, '1080')
    await userEvent.tab()
    await waitFor(() => expect(saved()).toEqual(expect.objectContaining({ route: { kind: 'proxy', proxyKind: 'socks5', host: 'proxy.example.com', port: 1080 } })))
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
    await choose('代理类型', 'HTTP')
    await waitFor(() => expect(saved()).toEqual(expect.objectContaining({ route: { kind: 'proxy', proxyKind: 'httpConnect', host: 'proxy.example.com', port: 1080 } })))
    await choose('网络', '直接连接')
    await waitFor(() => expect(saved()).toEqual(expect.objectContaining({ route: { kind: 'direct' } })))
    expect(screen.queryByLabelText('代理地址')).not.toBeInTheDocument()
  })
})

describe('translation settings: translating and foreign-language mail', () => {
  it('picks the target language by its own name and toggles the switches', async () => {
    const { saved } = await setup()
    expect(await screen.findByRole('combobox', { name: '翻译成' })).toHaveTextContent('跟随界面语言')
    await choose('翻译成', '日本語')
    await waitFor(() => expect(saved()).toEqual(expect.objectContaining({ targetLanguage: 'ja' })))
    const quotes = screen.getByRole('switch', { name: '翻译引用的历史邮件' })
    expect(quotes).not.toBeChecked()
    await userEvent.click(quotes)
    await waitFor(() => expect(saved()).toEqual(expect.objectContaining({ includeQuotes: true })))
    await userEvent.click(screen.getByRole('switch', { name: '提示翻译' }))
    await waitFor(() => expect(saved()).toEqual(expect.objectContaining({ suggest: false })))
  })

  it('edits the two language lists so a language is never in both', async () => {
    const { saved } = await setup({ neverSuggest: ['ja'] })
    const never = await screen.findByRole('combobox', { name: /不提示的语言/ })
    const always = screen.getByRole('combobox', { name: /总是翻译的语言/ })
    expect(screen.getByText('日语')).toBeInTheDocument()
    // Japanese is already taken by the other list, so it is not offered here.
    await userEvent.click(always)
    expect(screen.queryByRole('option', { name: '日语' })).not.toBeInTheDocument()
    await userEvent.click(await screen.findByRole('option', { name: '英语' }))
    await waitFor(() => expect(saved()).toEqual(expect.objectContaining({ alwaysTranslate: ['en'], neverSuggest: ['ja'] })))
    await userEvent.click(screen.getByRole('button', { name: '移除日语' }))
    await waitFor(() => expect(saved()).toEqual(expect.objectContaining({ neverSuggest: [], alwaysTranslate: ['en'] })))
    await userEvent.click(never)
    await userEvent.click(await screen.findByRole('option', { name: '日语' }))
    await waitFor(() => expect(saved()).toEqual(expect.objectContaining({ neverSuggest: ['ja'], alwaysTranslate: ['en'] })))
    // Adding a language that sits in the other list moves it.
    await userEvent.click(screen.getByRole('button', { name: '移除英语' }))
    await waitFor(() => expect(saved()).toEqual(expect.objectContaining({ alwaysTranslate: [] })))
  })
})

describe('translation settings: saved translations', () => {
  it('counts what is saved and refreshes after clearing', async () => {
    const { client } = await setup()
    client.translationCacheStats.mockResolvedValueOnce({ count: 12, bytes: Math.round(0.3 * 1024 * 1024) })
    client.translationCacheStats.mockResolvedValue({ count: 0, bytes: 0 })
    expect(await screen.findByText('12 封邮件，约 0.3 MB')).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '清除' }))
    expect(client.clearTranslations).toHaveBeenCalledOnce()
    expect(await screen.findByText('没有已保存的译文')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '清除' })).toBeDisabled()
  })

  it('says so when clearing fails', async () => {
    const { client } = await setup()
    client.translationCacheStats.mockResolvedValue({ count: 2, bytes: 5000 })
    client.clearTranslations.mockRejectedValueOnce(new Error('x'))
    expect(await screen.findByText('2 封邮件，约 不到 0.1 MB')).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '清除' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('无法清除')
  })
})

describe('translation settings page', () => {
  it('shows a read failure with a retry', async () => {
    const client = testClient()
    client.getSettings.mockRejectedValueOnce(new Error('x'))
    render(<TranslationPane client={client} />)
    expect(await screen.findByText('无法读取设置')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '重试' })).toBeInTheDocument()
  })

  it('reads in plain English', async () => {
    await i18n.changeLanguage('en')
    const { client } = await setup({ provider: 'deepl' }, [{ provider: 'deepl', configured: true }])
    client.translationCacheStats.mockResolvedValue({ count: 1, bytes: 300_000 })
    expect(await screen.findByRole('combobox', { name: 'Service' })).toHaveTextContent('DeepL')
    expect(screen.getByText('When you translate, the subject and text of the message are sent to the service you choose.')).toBeInTheDocument()
    expect(await screen.findByText('Saved')).toBeInTheDocument()
    expect(screen.getByRole('switch', { name: 'Offer to translate' })).toBeChecked()
    expect(await screen.findByText('1 message, about 0.3 MB')).toBeInTheDocument()
    await act(async () => undefined)
    expect(within(document.body).getByRole('combobox', { name: 'Translate into' })).toHaveTextContent('Same as the app language')
  })
})
