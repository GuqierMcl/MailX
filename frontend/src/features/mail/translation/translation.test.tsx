import { act, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/tooltip'
import { testClient } from '@/features/accounts/test-fixtures'
import { DEFAULT_TRANSLATION } from '@/features/settings/settings-queries'
import type { AiConsent, EmailDetail, TranslationInfo, TranslationProviderId, TranslationStatus } from '@/generated/accounts'
import { i18n } from '@/i18n'
import { MailxClientError } from '@/lib/mailx-client'
import { EmailView } from '../email-view'
import { TranslationConsentDialog } from './translation-consent-dialog'
import { useEmailTranslation } from './use-email-translation'

vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, convertFileSrc: () => 'mailx-body://localhost/' }))
afterEach(() => { vi.restoreAllMocks(); served.clear() })

function mail(id = 'mail-1', changes: { localOnly?: boolean; hasHtml?: boolean; subject?: string } = {}): EmailDetail {
  return {
    summary: {
      senderGroup: null, id, accountId: 'a', folderId: 'inbox', subject: changes.subject ?? `Subject ${id}`,
      from: { name: 'Yuki', address: 'yuki@example.com' }, receivedAt: '2026-10-28T00:00:00Z',
      isRead: true, isStarred: false, pendingChanges: false, matchSnippet: null, to: { addresses: [], total: 0 },
      localOnly: changes.localOnly ?? false, archived: false, hasAttachment: false, bodyState: 'ready', previewState: 'ready', preview: '',
    },
    to: [], cc: [], replyTo: [], attachments: [], bodyFailure: null,
    body: { text: 'hello', hasHtml: changes.hasHtml ?? true, truncated: false, remoteImagesBlocked: 0 },
  }
}

const statusOf = (changes: Partial<TranslationStatus> = {}): TranslationStatus => ({
  emailId: 'mail-1', targetLanguage: 'zh-Hans', detectedLanguage: 'ja', detectionReliable: true, suggested: true, autoTranslate: false, translation: null, ...changes,
})
const translatedInfo = (changes: Partial<TranslationInfo> = {}): TranslationInfo => ({ provider: 'deepl', sourceLanguage: 'ja', subject: 'Translated subject', partial: false, ...changes })
const failure = (code: 'translationCancelled' | 'translationNetwork' | 'translationNotConfigured' | 'translationUnavailable' | 'translationRateLimited' | 'translationKeyRejected' | 'aiModelNotFound' | 'aiRequestRejected' | 'notFound') => new MailxClientError({ code, message: 'x', field: null })

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: unknown) => void
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej })
  return { promise, resolve, reject }
}

const served = new Map<string, Partial<TranslationStatus>>()
/** The next translation produces this; later status calls report it as cached. */
function produces(client: ReturnType<typeof testClient>, result: TranslationStatus) {
  client.translateEmail.mockImplementation(async () => { served.set(result.emailId, { translation: result.translation, targetLanguage: result.targetLanguage }); return result })
}

async function setup(options: { provider?: TranslationProviderId | null; consented?: boolean; keyed?: boolean; status?: Partial<TranslationStatus> } = {}) {
  const { provider = 'deepl', consented = true, keyed = true } = options
  const client = testClient()
  await client.updateSettings({ translation: { ...DEFAULT_TRANSLATION, provider, consentedProvider: consented ? provider : null } })
  client.updateSettings.mockClear()
  client.translationKeys.mockResolvedValue(keyed && provider ? [{ provider, configured: true }] : [])
  // Like the core, the status reports what translating has produced so far.
  client.translationStatus.mockImplementation(async (emailId) => ({ ...statusOf({ emailId, ...options.status }), ...served.get(emailId) }))
  client.translateEmail.mockImplementation(async (emailId) => {
    throw new Error(`unexpected translation of ${emailId}`)
  })
  let serial = 0
  client.prepareBody.mockImplementation(async () => ({ documentToken: `token-${++serial}` }))
  return client
}

function Harness({ client, email, openSettings }: { client: ReturnType<typeof testClient>; email: EmailDetail; openSettings: () => void }) {
  const translation = useEmailTranslation(client, email, openSettings)
  return <TooltipProvider>
    <EmailView client={client} email={email} loading={false} error={null} downloading={false} onDownload={vi.fn()} onFlag={vi.fn()}
      onClose={vi.fn()} onRespond={vi.fn()} translation={translation} />
    <TranslationConsentDialog translation={translation} />
  </TooltipProvider>
}

async function mount(client: ReturnType<typeof testClient>, email = mail()) {
  const openSettings = vi.fn()
  const view = render(<Harness client={client} email={email} openSettings={openSettings} />)
  const show = (next: EmailDetail) => view.rerender(<Harness client={client} email={next} openSettings={openSettings} />)
  return { openSettings, show }
}

const bar = () => screen.findByRole('status', { name: /邮件翻译|Translation/ })
const original = () => screen.getByTitle(/^(邮件 HTML 正文|HTML message body|Message HTML body)$/)
const translatedFrame = () => screen.queryByTitle(/（译文）|\(translated\)/)

describe('translation notice', () => {
  it('offers to translate a message in another language, and stays out of the way otherwise', async () => {
    const client = await setup()
    await mount(client)
    const notice = await bar()
    expect(notice).toHaveTextContent('这封邮件是日语')
    expect(within(notice).getByRole('button', { name: '翻译成简体中文' })).toBeInTheDocument()
    expect(client.translateEmail).not.toHaveBeenCalled()
  })

  it('shows nothing when the status offers nothing', async () => {
    const client = await setup({ status: { suggested: false } })
    await mount(client)
    await waitFor(() => expect(client.translationStatus).toHaveBeenCalled())
    expect(screen.queryByRole('status', { name: '邮件翻译' })).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: '翻译' })).toBeInTheDocument()
  })

  it('dismisses the offer for this message and remembers it within the session', async () => {
    const client = await setup()
    const { show } = await mount(client)
    await userEvent.click(within(await bar()).getByRole('button', { name: '关闭翻译提示' }))
    expect(screen.queryByRole('status', { name: '邮件翻译' })).not.toBeInTheDocument()
    show(mail('mail-2'))
    expect(await bar()).toBeInTheDocument()
    show(mail('mail-1'))
    await waitFor(() => expect(client.translationStatus).toHaveBeenCalledTimes(3))
    expect(screen.queryByRole('status', { name: '邮件翻译' })).not.toBeInTheDocument()
  })

  it('is absent for device-only copies and messages without an HTML document', async () => {
    const client = await setup()
    const { show } = await mount(client, mail('local-sent:1', { localOnly: true }))
    expect(screen.queryByRole('button', { name: '翻译' })).not.toBeInTheDocument()
    show(mail('plain', { hasHtml: false }))
    expect(screen.queryByRole('button', { name: '翻译' })).not.toBeInTheDocument()
    expect(client.translationStatus).not.toHaveBeenCalled()
    expect(screen.queryByRole('status', { name: '邮件翻译' })).not.toBeInTheDocument()
  })
})

describe('translating', () => {
  it('goes from the offer through translating to the translation, keeping both documents', async () => {
    const client = await setup()
    const pending = deferred<TranslationStatus>()
    client.translateEmail.mockReturnValue(pending.promise)
    await mount(client)
    await userEvent.click(within(await bar()).getByRole('button', { name: '翻译成简体中文' }))
    expect(client.translateEmail).toHaveBeenCalledExactlyOnceWith('mail-1', undefined)
    expect(await bar()).toHaveTextContent('正在翻译…')
    // The original stays readable underneath, and the toolbar button reports being busy.
    expect(original()).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '翻译' })).toHaveAttribute('aria-busy', 'true')
    await act(async () => { pending.resolve(statusOf({ translation: translatedInfo() })) })
    const done = await bar()
    expect(done).toHaveTextContent('已从日语翻译')
    expect(done).toHaveTextContent('DeepL')
    await waitFor(() => expect(translatedFrame()).toBeInTheDocument())
    expect(client.prepareBody).toHaveBeenCalledWith('mail-1', false, 'zh-Hans')
    expect(translatedFrame()?.closest('[inert]')).toBeNull()
    expect(original().closest('[inert]')).not.toBeNull()
    expect(screen.getByRole('button', { name: '翻译' })).toHaveAttribute('aria-pressed', 'true')

    // Switching only toggles visibility: no document is prepared again and neither iframe unmounts.
    const originalFrame = original()
    await userEvent.click(within(done).getByRole('button', { name: '显示原文' }))
    expect(await screen.findByText('正在显示原文')).toBeInTheDocument()
    expect(original()).toBe(originalFrame)
    expect(translatedFrame()).toBeInTheDocument()
    expect(original().closest('[inert]')).toBeNull()
    expect(translatedFrame()?.closest('[inert]')).not.toBeNull()
    expect(client.prepareBody.mock.calls.filter(([, , target]) => target === undefined)).toHaveLength(1)
    expect(client.prepareBody).toHaveBeenCalledTimes(2)
    await userEvent.click(screen.getByRole('button', { name: '显示译文' }))
    expect(await screen.findByText(/已从日语翻译/)).toBeInTheDocument()
    expect(client.prepareBody).toHaveBeenCalledTimes(2)
    expect(client.translateEmail).toHaveBeenCalledTimes(1)
  })

  it('shows the translated subject with the original as its title, only while the translation shows', async () => {
    const client = await setup()
    produces(client, statusOf({ translation: translatedInfo() }))
    await mount(client, mail('mail-1', { subject: 'Original subject' }))
    const heading = screen.getByRole('heading', { level: 2 })
    expect(heading).toHaveTextContent('Original subject')
    await userEvent.click(within(await bar()).getByRole('button', { name: '翻译成简体中文' }))
    await waitFor(() => expect(heading).toHaveTextContent('Translated subject'))
    expect(heading).toHaveAttribute('title', 'Original subject')
    await userEvent.click(screen.getByRole('button', { name: '翻译' }))
    await waitFor(() => expect(heading).toHaveTextContent('Original subject'))
    expect(heading).not.toHaveAttribute('title')
    await userEvent.click(screen.getByRole('button', { name: '翻译' }))
    await waitFor(() => expect(heading).toHaveTextContent('Translated subject'))
  })

  it('cancels from the notice and returns to the offer without an error', async () => {
    const client = await setup()
    const pending = deferred<TranslationStatus>()
    client.translateEmail.mockReturnValue(pending.promise)
    await mount(client)
    await userEvent.click(within(await bar()).getByRole('button', { name: '翻译成简体中文' }))
    await userEvent.click(within(await bar()).getByRole('button', { name: '取消' }))
    expect(client.cancelTranslation).toHaveBeenCalledExactlyOnceWith('mail-1')
    await act(async () => { pending.reject(failure('translationCancelled')) })
    expect(await bar()).toHaveTextContent('这封邮件是日语')
    expect(screen.queryByText(/翻译已取消/)).not.toBeInTheDocument()
  })

  it('explains a failure calmly and retries', async () => {
    const client = await setup()
    client.translateEmail.mockRejectedValueOnce(failure('translationNetwork'))
    client.translateEmail.mockResolvedValueOnce(statusOf({ translation: translatedInfo() }))
    await mount(client)
    await userEvent.click(within(await bar()).getByRole('button', { name: '翻译成简体中文' }))
    const notice = await screen.findByText(/无法连接翻译服务/)
    expect(notice.closest('[role="status"]')).toHaveClass('bg-muted')
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '重试' }))
    expect(await screen.findByText(/已从日语翻译/)).toBeInTheDocument()
    expect(client.translateEmail).toHaveBeenCalledTimes(2)
  })

  it('points to the settings when the service is missing', async () => {
    const client = await setup()
    client.translateEmail.mockRejectedValueOnce(failure('translationNotConfigured'))
    const { openSettings } = await mount(client)
    await userEvent.click(within(await bar()).getByRole('button', { name: '翻译成简体中文' }))
    expect(await screen.findByText(/翻译已关闭，或所选服务还没有保存密钥/)).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '重试' })).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '去设置' }))
    expect(openSettings).toHaveBeenCalledOnce()
  })

  it('goes back to the original and says why when the translated document cannot be prepared', async () => {
    const client = await setup()
    produces(client, statusOf({ translation: translatedInfo() }))
    client.prepareBody.mockImplementation(async (_id, _images, target) => {
      if (target !== undefined) throw failure('notFound')
      return { documentToken: 'original-token' }
    })
    await mount(client)
    await userEvent.click(within(await bar()).getByRole('button', { name: '翻译成简体中文' }))
    await waitFor(() => expect(screen.getByRole('status', { name: '邮件翻译' })).toHaveTextContent('重试'))
    expect(translatedFrame()).not.toBeInTheDocument()
    expect(original().closest('[inert]')).toBeNull()
    expect(screen.getByRole('button', { name: '翻译' })).toHaveAttribute('aria-pressed', 'false')
  })
})

describe('keyless services', () => {
  it('translate with no saved key at all, asking for consent once', async () => {
    const client = await setup({ provider: 'microsoftFree', consented: false, keyed: false })
    produces(client, statusOf({ translation: translatedInfo({ provider: 'microsoftFree' }) }))
    await mount(client)
    await userEvent.click(within(await bar()).getByRole('button', { name: '翻译成简体中文' }))
    const dialog = await screen.findByRole('alertdialog')
    expect(dialog).toHaveTextContent('把邮件内容发送给Microsoft 翻译？')
    expect(dialog).toHaveTextContent('发送给Microsoft 翻译，')
    await userEvent.click(within(dialog).getByRole('button', { name: '翻译' }))
    expect(await screen.findByText(/已从日语翻译/)).toHaveTextContent('Microsoft 翻译')
    expect(client.updateSettings).toHaveBeenCalledOnce()
    expect(client.translateEmail).toHaveBeenCalledOnce()
    expect(screen.queryByText('还没有设置翻译服务')).not.toBeInTheDocument()
  })

  it('name Google in the consent sentence', async () => {
    const client = await setup({ provider: 'googleFree', consented: false, keyed: false })
    await mount(client)
    await userEvent.click(within(await bar()).getByRole('button', { name: '翻译成简体中文' }))
    expect(await screen.findByRole('alertdialog')).toHaveTextContent('发送给Google 翻译，')
  })

  it.each(['translationUnavailable', 'translationRateLimited', 'translationNetwork'] as const)('offer another service next to retry after %s', async (code) => {
    const client = await setup({ provider: 'microsoftFree', keyed: false })
    client.translateEmail.mockRejectedValueOnce(failure(code))
    const { openSettings } = await mount(client)
    await userEvent.click(within(await bar()).getByRole('button', { name: '翻译成简体中文' }))
    const notice = await screen.findByRole('status', { name: '邮件翻译' })
    await waitFor(() => expect(within(notice).getByRole('button', { name: '重试' })).toBeInTheDocument())
    await userEvent.click(within(notice).getByRole('button', { name: '换一种翻译服务' }))
    expect(openSettings).toHaveBeenCalledOnce()
  })

  it.each([['aiModelNotFound', '找不到这个模型'], ['aiRequestRejected', '服务商拒绝了这次请求']] as const)('show the AI error %s and offer another service', async (code, text) => {
    const client = await setupAi({ consentedAi: { connectionId: 'c1', baseUrl: 'https://api.example/v1' } })
    client.translateEmail.mockRejectedValueOnce(failure(code))
    const { openSettings } = await mount(client)
    await userEvent.click(within(await bar()).getByRole('button', { name: '翻译成简体中文' }))
    const notice = await screen.findByRole('status', { name: '邮件翻译' })
    await waitFor(() => expect(notice).toHaveTextContent(text))
    await userEvent.click(within(notice).getByRole('button', { name: '换一种翻译服务' }))
    expect(openSettings).toHaveBeenCalledOnce()
  })

  it('do not offer another service for a keyed service that fails', async () => {
    const client = await setup()
    client.translateEmail.mockRejectedValueOnce(failure('translationUnavailable'))
    await mount(client)
    await userEvent.click(within(await bar()).getByRole('button', { name: '翻译成简体中文' }))
    await waitFor(() => expect(screen.getByRole('status', { name: '邮件翻译' })).toHaveTextContent('重试'))
    expect(screen.queryByRole('button', { name: '换一种翻译服务' })).not.toBeInTheDocument()
  })

  it('show the toolbar button without a setup popover', async () => {
    const client = await setup({ provider: 'googleFree', keyed: false, status: { suggested: false } })
    produces(client, statusOf({ translation: translatedInfo({ provider: 'googleFree' }) }))
    await mount(client)
    await userEvent.click(await screen.findByRole('button', { name: '翻译' }))
    expect(screen.queryByText('还没有设置翻译服务')).not.toBeInTheDocument()
    await waitFor(() => expect(client.translateEmail).toHaveBeenCalledOnce())
  })
})

describe('no translation service', () => {
  it('explains it in a small popover from the toolbar button and opens the settings', async () => {
    const client = await setup({ provider: null, status: { suggested: false } })
    const { openSettings } = await mount(client)
    await userEvent.click(await screen.findByRole('button', { name: '翻译' }))
    expect(await screen.findByText('还没有设置翻译服务')).toBeInTheDocument()
    expect(client.translateEmail).not.toHaveBeenCalled()
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '去设置' }))
    expect(openSettings).toHaveBeenCalledOnce()
    await waitFor(() => expect(screen.queryByText('还没有设置翻译服务')).not.toBeInTheDocument())
  })
})

async function setupAi(options: { consentedAi?: AiConsent | null; baseUrl?: string } = {}) {
  const client = await setup({ provider: 'ai', consented: options.consentedAi !== undefined, keyed: false })
  const baseUrl = options.baseUrl ?? 'https://api.example/v1'
  client.aiState.mockResolvedValue({
    connections: [{ id: 'c1', providerId: 'custom', name: 'Acme AI', protocol: 'openaiCompatible', baseUrl, useProxy: false, hasKey: true }],
    defaultModel: { connectionId: 'c1', modelId: 'm1', name: 'M1', contextLength: null, outputLength: null }, translationModel: null,
    route: { kind: 'direct' }, proxyPasswordSet: false,
  })
  if (options.consentedAi !== undefined) await client.updateSettings({ translation: { ...DEFAULT_TRANSLATION, provider: 'ai', consentedProvider: 'ai', consentedAi: options.consentedAi } })
  client.updateSettings.mockClear()
  return client
}

describe('consent for the AI service', () => {
  it('names the connection that gets the text and saves it with the consent', async () => {
    const client = await setupAi()
    produces(client, statusOf({ translation: translatedInfo({ provider: 'ai' }) }))
    await mount(client)
    await userEvent.click(within(await bar()).getByRole('button', { name: '翻译成简体中文' }))
    const dialog = await screen.findByRole('alertdialog')
    expect(dialog).toHaveTextContent('把邮件内容发送给Acme AI？')
    expect(dialog).toHaveTextContent('邮件的主题和正文会发送给Acme AI')
    await userEvent.click(within(dialog).getByRole('button', { name: '翻译' }))
    await waitFor(() => expect(client.translateEmail).toHaveBeenCalledOnce())
    expect(client.updateSettings).toHaveBeenCalledWith({ translation: expect.objectContaining({
      provider: 'ai', consentedProvider: 'ai', consentedAi: { connectionId: 'c1', baseUrl: 'https://api.example/v1' } }) })
  })

  it('does not ask again for the same connection and address', async () => {
    const client = await setupAi({ consentedAi: { connectionId: 'c1', baseUrl: 'https://api.example/v1' } })
    produces(client, statusOf({ translation: translatedInfo({ provider: 'ai' }) }))
    await mount(client)
    await userEvent.click(within(await bar()).getByRole('button', { name: '翻译成简体中文' }))
    await waitFor(() => expect(client.translateEmail).toHaveBeenCalledOnce())
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
  })

  it('asks again when the connection address changed, before any request', async () => {
    const client = await setupAi({ consentedAi: { connectionId: 'c1', baseUrl: 'https://old.example/v1' } })
    await mount(client)
    await userEvent.click(within(await bar()).getByRole('button', { name: '翻译成简体中文' }))
    expect(await screen.findByRole('alertdialog')).toHaveTextContent('把邮件内容发送给Acme AI？')
    expect(client.translateEmail).not.toHaveBeenCalled()
  })

  it('leaves consentedAi alone when agreeing to another service', async () => {
    const client = await setup({ provider: 'deepl', consented: false })
    produces(client, statusOf({ translation: translatedInfo() }))
    await mount(client)
    await userEvent.click(within(await bar()).getByRole('button', { name: '翻译成简体中文' }))
    await userEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: '翻译' }))
    await waitFor(() => expect(client.updateSettings).toHaveBeenCalled())
    expect(client.updateSettings.mock.calls[0][0].translation).toEqual(expect.objectContaining({ consentedProvider: 'deepl', consentedAi: null }))
  })
})

describe('consent', () => {
  it('asks before the first translation, leaves the settings alone when declined, and saves the consent when agreed', async () => {
    const client = await setup({ consented: false })
    produces(client, statusOf({ translation: translatedInfo() }))
    await mount(client)
    await userEvent.click(within(await bar()).getByRole('button', { name: '翻译成简体中文' }))
    const dialog = await screen.findByRole('alertdialog')
    expect(dialog).toHaveTextContent('把邮件内容发送给DeepL？')
    expect(dialog).toHaveTextContent('附件和图片不会发送')
    await waitFor(() => expect(within(dialog).getByRole('button', { name: '取消' })).toHaveFocus())
    await userEvent.click(within(dialog).getByRole('button', { name: '取消' }))
    await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument())
    expect(client.updateSettings).not.toHaveBeenCalled()
    expect(client.translateEmail).not.toHaveBeenCalled()

    await userEvent.click(within(await bar()).getByRole('button', { name: '翻译成简体中文' }))
    await userEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: '翻译' }))
    await waitFor(() => expect(client.translateEmail).toHaveBeenCalledOnce())
    expect(client.updateSettings).toHaveBeenCalledWith({ translation: expect.objectContaining({ provider: 'deepl', consentedProvider: 'deepl' }) })
    expect(client.updateSettings.mock.invocationCallOrder[0]).toBeLessThan(client.translateEmail.mock.invocationCallOrder[0])
    expect(await screen.findByText(/已从日语翻译/)).toBeInTheDocument()

    // Agreed once: translating again does not ask again.
    await userEvent.click(screen.getByRole('button', { name: '翻译选项' }))
    await userEvent.click(await screen.findByRole('menuitem', { name: '重新翻译' }))
    await waitFor(() => expect(client.translateEmail).toHaveBeenCalledTimes(2))
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument()
    expect(client.clearTranslation).toHaveBeenCalledWith('mail-1')
  })
})

describe('cached and automatic translation', () => {
  it('shows a cached translation without calling the translation service', async () => {
    const client = await setup({ status: { translation: translatedInfo() } })
    await mount(client)
    const notice = await bar()
    expect(notice).toHaveTextContent('这封邮件是日语')
    await userEvent.click(within(notice).getByRole('button', { name: '显示译文' }))
    expect(await screen.findByText(/已从日语翻译/)).toBeInTheDocument()
    await waitFor(() => expect(client.prepareBody).toHaveBeenCalledWith('mail-1', false, 'zh-Hans'))
    expect(client.translateEmail).not.toHaveBeenCalled()
  })

  it('translates on open when the language is on the always list, until the user returns to the original', async () => {
    const client = await setup({ status: { autoTranslate: true } })
    produces(client, statusOf({ autoTranslate: true, translation: translatedInfo() }))
    const { show } = await mount(client)
    expect(await screen.findByText(/已从日语翻译/)).toBeInTheDocument()
    expect(client.translateEmail).toHaveBeenCalledOnce()
    await userEvent.click(screen.getByRole('button', { name: '显示原文' }))
    show(mail('mail-2'))
    await waitFor(() => expect(client.translateEmail).toHaveBeenCalledTimes(2))
    show(mail('mail-1'))
    await waitFor(() => expect(client.translationStatus).toHaveBeenCalledTimes(3))
    await act(async () => { await Promise.resolve() })
    // Back on the first message the user's choice for it holds: nothing is translated on its own again.
    expect(client.translateEmail).toHaveBeenCalledTimes(2)
    expect(client.translateEmail.mock.calls.map(([id]) => id)).toEqual(['mail-1', 'mail-2'])
  })
})

describe('moving between messages', () => {
  it('ignores a result that arrives after another message was opened, and cancels the work', async () => {
    const client = await setup()
    const pending = deferred<TranslationStatus>()
    client.translateEmail.mockReturnValue(pending.promise)
    const { show } = await mount(client)
    await userEvent.click(within(await bar()).getByRole('button', { name: '翻译成简体中文' }))
    show(mail('mail-2'))
    await waitFor(() => expect(client.cancelTranslation).toHaveBeenCalledWith('mail-1'))
    await act(async () => { pending.resolve(statusOf({ emailId: 'mail-1', translation: translatedInfo() })) })
    expect(await bar()).toHaveTextContent('这封邮件是日语')
    expect(screen.queryByText(/已从日语翻译/)).not.toBeInTheDocument()
    expect(translatedFrame()).not.toBeInTheDocument()
  })
})

describe('language lists in the menu', () => {
  it('makes "always translate" and "don\'t suggest" exclusive for the detected language', async () => {
    const client = await setup()
    await mount(client)
    await screen.findByRole('button', { name: '翻译成简体中文' })
    await userEvent.click(screen.getByRole('button', { name: '翻译选项' }))
    await userEvent.click(await screen.findByRole('menuitemcheckbox', { name: '总是翻译日语' }))
    await waitFor(() => expect(client.updateSettings).toHaveBeenLastCalledWith({ translation: expect.objectContaining({ alwaysTranslate: ['ja'], neverSuggest: [] }) }))
    // The menu stays open for a checkable item, so the next one can be chosen straight away.
    await waitFor(() => expect(screen.getByRole('menuitemcheckbox', { name: '总是翻译日语' })).toBeChecked())
    await userEvent.click(screen.getByRole('menuitemcheckbox', { name: '不再提示日语' }))
    await waitFor(() => expect(client.updateSettings).toHaveBeenLastCalledWith({ translation: expect.objectContaining({ alwaysTranslate: [], neverSuggest: ['ja'] }) }))
  })

  it('translates into the chosen language and checks the current one', async () => {
    const client = await setup()
    produces(client, statusOf({ targetLanguage: 'en', translation: translatedInfo() }))
    await mount(client)
    await screen.findByRole('button', { name: '翻译成简体中文' })
    await userEvent.click(screen.getByRole('button', { name: '翻译选项' }))
    await userEvent.click(await screen.findByRole('menuitem', { name: /翻译成/ }))
    expect(await screen.findByRole('menuitemradio', { name: '简体中文' })).toBeChecked()
    await userEvent.click(screen.getByRole('menuitemradio', { name: 'English' }))
    await waitFor(() => expect(client.translateEmail).toHaveBeenCalledWith('mail-1', 'en'))
    await waitFor(() => expect(client.prepareBody).toHaveBeenCalledWith('mail-1', false, 'en'))
  })
})

describe('in English', () => {
  it('words the notice, the translation and the consent in plain English', async () => {
    await i18n.changeLanguage('en')
    const client = await setup({ consented: false })
    produces(client, statusOf({ translation: translatedInfo() }))
    await mount(client)
    const notice = await screen.findByRole('status', { name: 'Translation' })
    expect(notice).toHaveTextContent('This message is in Japanese')
    await userEvent.click(within(notice).getByRole('button', { name: 'Translate to Simplified Chinese' }))
    const dialog = await screen.findByRole('alertdialog')
    expect(dialog).toHaveTextContent('Send this message to DeepL?')
    await userEvent.click(within(dialog).getByRole('button', { name: 'Translate' }))
    expect(await screen.findByText(/Translated from Japanese/)).toBeInTheDocument()
    expect(screen.getByRole('status', { name: 'Translation' })).toHaveTextContent('DeepL')
    await userEvent.click(screen.getByRole('button', { name: 'Show original' }))
    expect(await screen.findByText('Showing the original')).toBeInTheDocument()
  })
})

describe('keyless services in English', () => {
  it('words the consent sentence and the failure buttons in plain English', async () => {
    await i18n.changeLanguage('en')
    const client = await setup({ provider: 'googleFree', consented: false, keyed: false })
    client.translateEmail.mockRejectedValueOnce(failure('translationNetwork'))
    const { openSettings } = await mount(client)
    const notice = await screen.findByRole('status', { name: 'Translation' })
    await userEvent.click(within(notice).getByRole('button', { name: 'Translate to Simplified Chinese' }))
    const dialog = await screen.findByRole('alertdialog')
    expect(dialog).toHaveTextContent('Send this message to Google Translate?')
    expect(dialog).toHaveTextContent("are sent to Google Translate.")
    await userEvent.click(within(dialog).getByRole('button', { name: 'Translate' }))
    const failed = await screen.findByRole('status', { name: 'Translation' })
    await waitFor(() => expect(within(failed).getByRole('button', { name: 'Retry' })).toBeInTheDocument())
    await userEvent.click(within(failed).getByRole('button', { name: 'Use another service' }))
    expect(openSettings).toHaveBeenCalledOnce()
  })
})
