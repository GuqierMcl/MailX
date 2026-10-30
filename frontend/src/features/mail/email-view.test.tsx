import { act, render, screen } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/tooltip'
import type { MailxClient } from '@/lib/mailx-client'
import type { EmailDetail } from '@/generated/accounts'
import { exampleAccount, testClient } from '@/features/accounts/test-fixtures'
import { EmailView } from './email-view'
import { i18n } from '@/i18n'

function setup() {
  render(
    <TooltipProvider>
      <EmailView
        client={{} as MailxClient} email={null} loading error={null} downloading={false}
        onDownload={vi.fn()} onFlag={vi.fn()} onClose={vi.fn()} onRespond={vi.fn()}
      />
    </TooltipProvider>,
  )
}

async function focus(name: string) {
  const target = screen.getByRole('button', { name })
  // Keyboard focus is how a disabled button's tooltip is reached (its wrapper takes the tab stop).
  for (let i = 0; i < 40; i++) {
    await userEvent.tab()
    if (document.activeElement === target || document.activeElement?.contains(target)) return
  }
  throw new Error(`could not focus ${name}`)
}

describe('reader toolbar tooltips', () => {
  it('updates actions that were previously module-level labels when the language changes', async () => {
    setup()
    await act(async () => { await i18n.changeLanguage('en') })
    expect(screen.getByRole('toolbar', { name: 'Message actions' })).toBeInTheDocument()
    for (const name of ['Archive', 'Reply', 'Reply all', 'Forward', 'Mark as unread', 'Add star', 'Close message']) {
      expect(screen.getByRole('button', { name })).toBeInTheDocument()
    }
    expect(screen.queryByRole('button', { name: '回复' })).not.toBeInTheDocument()
  })

  it('uses tooltips, not native titles, and names every icon button', () => {
    setup()
    const bar = screen.getByRole('toolbar', { name: '邮件操作' })
    expect(bar.querySelectorAll('[title]')).toHaveLength(0)
    for (const name of ['归档', '回复', '全部回复', '转发', '标为未读', '加星标', '关闭邮件']) {
      expect(screen.getByRole('button', { name })).toBeInTheDocument()
    }
  })

  it('explains a not-yet-implemented (disabled) button on focus', async () => {
    setup()
    await focus('归档')
    expect(await screen.findByText('归档（尚未实现）')).toBeInTheDocument()
  })

  it('shows the Esc shortcut on the close button', async () => {
    setup()
    await focus('关闭邮件')
    expect(await screen.findByText('关闭邮件（Esc）')).toBeInTheDocument()
  })
})

describe('body failure guidance', () => {
  it.each([
    ['providerPolicy', 'netease163', /阻止了正文下载/],
    ['providerPolicy', 'netease126', /阻止了正文下载/],
    ['providerPolicy', 'neteaseYeah', /阻止了正文下载/],
    ['providerPolicy', null, /阻止了正文下载/],
    ['authentication', 'netease163', /邮箱认证失败/],
  ] as const)('explains %s for %s and retries only when the reader asks', async (bodyFailure, providerId, expectedHint) => {
    const account = exampleAccount({ providerId })
    const email: EmailDetail = {
      summary: {
        id: 'policy-mail', accountId: account.id, folderId: 'inbox', subject: 'Policy failure',
        from: { name: null, address: 'synthetic@example.com' }, receivedAt: '2026-10-28T00:00:00Z',
        isRead: false, isStarred: false, pendingChanges: false, to: { addresses: [], total: 0 }, localOnly: false, hasAttachment: false, bodyState: 'failed', previewState: 'failed', preview: '',
      },
      to: [], cc: [], replyTo: [], attachments: [], body: null, bodyFailure,
    }
    const onDownload = vi.fn()
    const client = testClient([account])
    render(<TooltipProvider><EmailView client={client} email={email} providerId={providerId} loading={false} error={null} downloading={false}
      onDownload={onDownload} onFlag={vi.fn()} onClose={vi.fn()} onRespond={vi.fn()} /></TooltipProvider>)
    expect(screen.getByText('正文获取失败')).toBeInTheDocument()
    expect(screen.getByText(expectedHint)).toBeInTheDocument()
    if (bodyFailure === 'providerPolicy') expect(screen.queryByText(/邮箱认证失败/)).not.toBeInTheDocument()
    expect(onDownload).not.toHaveBeenCalled()
    if (bodyFailure === 'providerPolicy' && providerId) {
      await userEvent.click(screen.getByRole('button', { name: '查看官方帮助' }))
      expect(client.openProviderHelp).toHaveBeenCalledWith(providerId)
    } else {
      expect(screen.queryByRole('button', { name: '查看官方帮助' })).not.toBeInTheDocument()
      expect(client.openProviderHelp).not.toHaveBeenCalled()
    }
    await userEvent.click(screen.getByRole('button', { name: '重试' }))
    expect(onDownload).toHaveBeenCalledOnce()
  })
})

describe('local-only sent copy', () => {
  function localEmail(localOnly: boolean): EmailDetail {
    return {
      summary: {
        id: localOnly ? 'local-sent:1' : 'sent-1', accountId: 'a', folderId: 'sent', subject: 'Sent copy',
        from: { name: null, address: 'me@example.com' }, receivedAt: '2026-10-28T00:00:00Z',
        isRead: true, isStarred: false, pendingChanges: false, to: { addresses: [{ name: null, address: 'you@example.com' }], total: 1 }, localOnly,
        hasAttachment: false, bodyState: 'ready', previewState: 'ready', preview: '',
      },
      to: [{ name: null, address: 'you@example.com' }], cc: [], replyTo: [], attachments: [], body: { kind: 'text', text: 'hi' } as never, bodyFailure: null,
    }
  }
  function mount(email: EmailDetail, extra: { onRemoveLocal?: (e: EmailDetail['summary']) => void, onRespond?: () => void } = {}) {
    render(<TooltipProvider><EmailView client={testClient([exampleAccount()])} email={email} providerId={null} loading={false} error={null} downloading={false}
      onDownload={vi.fn()} onFlag={vi.fn()} onClose={vi.fn()} onRespond={extra.onRespond ?? vi.fn()} onRemoveLocal={extra.onRemoveLocal} /></TooltipProvider>)
  }

  it('hides read and star actions, shows the marker, keeps reply actions', () => {
    mount(localEmail(true), { onRemoveLocal: vi.fn() })
    expect(screen.queryByRole('button', { name: /标为(已读|未读)/ })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /星标/ })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '删除' })).not.toBeInTheDocument()
    expect(screen.getByRole('img', { name: '仅在本机' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: '回复' })).toBeInTheDocument()
  })

  it('keeps flag actions and no marker for a server message', () => {
    mount(localEmail(false), { onRemoveLocal: vi.fn() })
    expect(screen.getByRole('button', { name: /标为未读/ })).toBeInTheDocument()
    expect(screen.queryByRole('img', { name: '仅在本机' })).not.toBeInTheDocument()
    expect(screen.queryByRole('button', { name: '从本机移除' })).not.toBeInTheDocument()
  })

  it('removes only after the confirm dialog is accepted', async () => {
    const onRemoveLocal = vi.fn()
    mount(localEmail(true), { onRemoveLocal })
    await userEvent.click(screen.getByRole('button', { name: '从本机移除' }))
    expect(await screen.findByText(/不影响服务器，且无法撤销/)).toBeInTheDocument()
    expect(onRemoveLocal).not.toHaveBeenCalled()
    await userEvent.click(screen.getByRole('button', { name: '取消' }))
    expect(onRemoveLocal).not.toHaveBeenCalled()
    await userEvent.click(screen.getByRole('button', { name: '从本机移除' }))
    await userEvent.click(await screen.findByRole('button', { name: '移除' }))
    expect(onRemoveLocal).toHaveBeenCalledWith(expect.objectContaining({ id: 'local-sent:1' }))
  })
})
