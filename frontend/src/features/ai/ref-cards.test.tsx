import { describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { render, screen, waitFor } from '@/test/render'
import { TooltipProvider } from '@/components/ui/tooltip'
import { exampleAccount, testClient } from '@/features/accounts/test-fixtures'
import type { EmailDetail, EmailSummary, Folder } from '@/generated/accounts'
import { ChatEnvContext, type ChatEnv } from './chat-env'
import { Markdown } from './markdown-view'

const account = exampleAccount({ displayName: '工作邮箱' })
const MAIL = '3f2b8c1e-5d4a-4c7b-9e10-aa11bb22cc33'
const FOLDER = '9a8b7c6d-1111-4222-8333-444455556666'

const summary = (over: Partial<EmailSummary> = {}): EmailSummary => ({
  senderGroup: null, id: MAIL, accountId: account.id, folderId: FOLDER, subject: '第三季度账单', from: { name: '王五', address: 'wang@example.com' }, preview: '',
  receivedAt: new Date(Date.now() - 3 * 3600_000).toISOString(), isRead: false, isStarred: false, hasAttachment: false, bodyState: 'ready', previewState: 'ready',
  pendingChanges: false, matchSnippet: null, to: { addresses: [], total: 0 }, localOnly: false, archived: false, ...over,
})
const detail = (over: Partial<EmailSummary> = {}): EmailDetail => ({ summary: summary(over), to: [], cc: [], replyTo: [], body: null, bodyFailure: null, attachments: [] })
const folder = (over: Partial<Folder> = {}): Folder => ({
  id: FOLDER, accountId: account.id, parentId: null, name: 'INBOX', role: 'inbox', selectable: true, countsKnown: true, unreadCount: 4, totalCount: 10, archivedCount: 0, archivedUnreadCount: 0, ...over,
})

function setup(text: string, { email = detail(), folders = [folder()] }: { email?: EmailDetail | Error; folders?: Folder[] } = {}) {
  const client = testClient([account])
  client.getEmail.mockImplementation(async () => { if (email instanceof Error) throw email; return email })
  const env: ChatEnv = {
    client, accounts: [account], folders, onOpenEmail: vi.fn(), onOpenDraft: vi.fn(), onOpenFolder: vi.fn(), respond: vi.fn(), retry: vi.fn(), focusInput: vi.fn(),
    undone: new Set(), markUndone: vi.fn(),
  }
  render(<TooltipProvider><ChatEnvContext value={env}><Markdown text={text} onOpenLink={vi.fn()} /></ChatEnvContext></TooltipProvider>)
  return { client, env }
}

describe('mail and folder references in a reply', () => {
  it('draws a mail card from local data, not from the model\'s label, and opens the mail', async () => {
    const { env } = setup(`[一封别的邮件](mailx://email/${MAIL})`)
    const card = await screen.findByRole('button', { name: '打开邮件：第三季度账单' })
    expect(card).toHaveTextContent('王五')
    expect(card).toHaveTextContent('第三季度账单')
    expect(card).toHaveTextContent('3小时前')
    expect(card.querySelector('[data-slot="unread-dot"]')).toBeInTheDocument()
    expect(screen.queryByText('一封别的邮件')).not.toBeInTheDocument()
    await userEvent.click(card)
    expect(env.onOpenEmail).toHaveBeenCalledWith({ id: MAIL, accountId: account.id })
  })

  it('shows no unread dot for a mail that was read', async () => {
    setup(`[x](mailx://email/${MAIL})`, { email: detail({ isRead: true }) })
    const card = await screen.findByRole('button', { name: '打开邮件：第三季度账单' })
    expect(card.querySelector('[data-slot="unread-dot"]')).not.toBeInTheDocument()
  })

  it('draws a chip inside a sentence', async () => {
    const { env } = setup(`先看[这封](mailx://email/${MAIL})吧。`)
    const chip = await screen.findByRole('button', { name: '打开邮件：第三季度账单' })
    expect(chip.closest('p')).toHaveTextContent(/先看.?第三季度账单吧。/)
    expect(chip).not.toHaveTextContent('王五')
    await userEvent.click(chip)
    expect(env.onOpenEmail).toHaveBeenCalledTimes(1)
  })

  it('falls back to the label as plain text when the mail cannot be found', async () => {
    const { client } = setup(`先看[旧邮件](mailx://email/${MAIL})吧。\n\n[另一封](mailx://email/${MAIL})`, { email: new Error('not found') })
    await waitFor(() => expect(client.getEmail).toHaveBeenCalled())
    expect(await screen.findByText('先看旧邮件吧。')).toBeInTheDocument()
    expect(await screen.findByText('另一封')).toBeInTheDocument()
    expect(screen.queryByRole('button')).not.toBeInTheDocument()
    expect(screen.queryByRole('link')).not.toBeInTheDocument()
  })

  it('draws a folder card with the sidebar\'s name, the account and the unread count, and opens the folder', async () => {
    const { env } = setup(`[随便叫什么](mailx://folder/${FOLDER})`)
    const card = await screen.findByRole('button', { name: '打开文件夹：收件箱' })
    expect(card).toHaveTextContent('收件箱')
    expect(card).toHaveTextContent('工作邮箱')
    expect(card).toHaveTextContent('4')
    await userEvent.click(card)
    expect(env.onOpenFolder).toHaveBeenCalledWith(folder())
  })

  it('draws a folder chip inside a sentence', async () => {
    setup(`去[那里](mailx://folder/${FOLDER})看。`)
    const chip = await screen.findByRole('button', { name: '打开文件夹：收件箱' })
    expect(chip.closest('p')).toHaveTextContent('去收件箱看。')
  })

  it('keeps an unknown folder as plain text', () => {
    setup(`[不存在](mailx://folder/${FOLDER})`, { folders: [] })
    expect(screen.getByText('不存在')).toBeInTheDocument()
    expect(screen.queryByRole('button')).not.toBeInTheDocument()
  })

  it('keeps an unselectable folder as plain text', () => {
    setup(`[容器](mailx://folder/${FOLDER})`, { folders: [folder({ selectable: false })] })
    expect(screen.getByText('容器')).toBeInTheDocument()
    expect(screen.queryByRole('button')).not.toBeInTheDocument()
  })

  it('does not look up an id that is not well formed', () => {
    const { client } = setup('[坏的](mailx://email/a b)和[另一个](mailx://email/../x)')
    expect(client.getEmail).not.toHaveBeenCalled()
    expect(screen.getByText('坏的和另一个')).toBeInTheDocument()
  })
})
