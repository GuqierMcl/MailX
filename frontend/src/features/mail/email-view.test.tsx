import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/tooltip'
import type { MailxClient } from '@/lib/mailx-client'
import { EmailView } from './email-view'

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
