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
        isRead: false, isStarred: false, pendingChanges: false, hasAttachment: false, bodyState: 'failed', preview: '',
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
