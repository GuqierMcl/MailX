import { useState } from 'react'
import { fireEvent, render, screen } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'
import { ProviderPicker } from './provider-picker'
import { GENERIC_PROVIDER, MICROSOFT_WORK_PROVIDER, PROVIDERS, type MailProvider } from './providers'
import { i18n } from '@/i18n'

const providers: MailProvider[] = [
  GENERIC_PROVIDER,
  { ...GENERIC_PROVIDER, id: 'example', name: 'Example Mail', domain: 'example.test', description: 'Work email', searchAliases: ['示例', 'company'] },
  { ...GENERIC_PROVIDER, id: 'another', name: 'Another Mail', domain: 'another.test', description: 'Personal email', searchAliases: ['个人'] },
]

function PickerHarness({ onAdvance }: { onAdvance(provider: MailProvider): void }) {
  const [selected, setSelected] = useState<MailProvider | null>(null)
  const [query, setQuery] = useState('')
  return <ProviderPicker providers={providers} selected={selected} query={query} onQueryChange={setQuery} onSelect={setSelected} onAdvance={onAdvance} />
}

describe('provider picker', () => {
  it.each(['  eXamPLE  ', 'EXAMPLE.TEST', 'Work email', 'company', '示例', 'example company'])('matches names, domains, descriptions and aliases: %s', (query) => {
    render(<PickerHarness onAdvance={vi.fn()} />)
    fireEvent.change(screen.getByRole('searchbox'), { target: { value: query } })
    expect(screen.getAllByRole('radio')).toHaveLength(1)
    expect(screen.getByRole('radio', { name: 'Example Mail' })).toBeInTheDocument()
  })

  it('retains a visible choice while searching and does not advance from the search input', async () => {
    const onAdvance = vi.fn()
    render(<PickerHarness onAdvance={onAdvance} />)
    await userEvent.click(screen.getByRole('radio', { name: 'Example Mail' }))
    await userEvent.type(screen.getByRole('searchbox'), 'mail{Enter}')
    expect(screen.getByRole('radio', { name: 'Example Mail' })).toBeChecked()
    expect(onAdvance).not.toHaveBeenCalled()
  })

  it('supports Space, arrow navigation and a single tab stop for the radio group', async () => {
    render(<PickerHarness onAdvance={vi.fn()} />)
    const user = userEvent.setup()
    await user.click(screen.getByRole('searchbox'))
    await user.tab()
    const generic = screen.getByRole('radio', { name: 'IMAP/SMTP' })
    expect(generic).toHaveFocus()
    await user.keyboard(' ')
    expect(generic).toBeChecked()
    await user.keyboard('{ArrowDown}')
    expect(screen.getByRole('radio', { name: 'Example Mail' })).toBeChecked()
    expect(screen.getByRole('radio', { name: 'Example Mail' })).toHaveFocus()
    await user.keyboard('{ArrowUp}{ArrowUp}')
    expect(screen.getByRole('radio', { name: 'Another Mail' })).toBeChecked()
    expect(screen.getByRole('radio', { name: 'Another Mail' })).toHaveFocus()
  })

  it.each(['zh-CN', 'en'])('keeps the whole Microsoft 365 / Exchange name in the list (%s) by letting names wrap instead of cutting them off', async (language) => {
    await i18n.changeLanguage(language)
    function Real() {
      const [selected, setSelected] = useState<MailProvider | null>(null)
      const [query, setQuery] = useState('')
      return <ProviderPicker selected={selected} query={query} onQueryChange={setQuery} onSelect={setSelected} onAdvance={vi.fn()} />
    }
    render(<Real />)
    const option = screen.getByRole('radio', { name: MICROSOFT_WORK_PROVIDER.name })
    const name = document.getElementById(option.getAttribute('aria-labelledby')!)!
    expect(name).toHaveTextContent('Microsoft 365 / Exchange')
    // `truncate` is what produced "Microsoft 365 / E..."; names wrap onto a second line instead.
    expect(name).not.toHaveClass('truncate')
    expect(name).toHaveClass('line-clamp-2', 'break-words')
    // Every row keeps the same wrapping rule, so no entry is cut off on one line.
    for (const provider of PROVIDERS) {
      const row = screen.getByRole('radio', { name: provider.name })
      expect(document.getElementById(row.getAttribute('aria-labelledby')!)).not.toHaveClass('truncate')
    }
    await i18n.changeLanguage('zh-CN')
  })
})
