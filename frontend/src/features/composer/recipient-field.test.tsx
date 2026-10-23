import { render, screen } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { useState } from 'react'
import { describe, expect, it } from 'vitest'
import type { Recipient } from './address-parse'
import { RecipientField } from './recipient-field'

function Harness({ initial = [] }: { initial?: Recipient[] }) {
  const [value, setValue] = useState<Recipient[]>(initial)
  return (
    <div>
      <RecipientField label="收件人" value={value} onChange={setValue} />
      <output aria-label="addresses">{value.map((item) => `${item.valid ? '' : '!'}${item.address}`).join('|')}</output>
      <button type="button">elsewhere</button>
    </div>
  )
}

const addresses = () => screen.getByLabelText('addresses').textContent

describe('recipient chips', () => {
  it('commits on Enter, comma, semicolon and blur', async () => {
    render(<Harness />)
    const input = screen.getByRole('textbox', { name: '收件人' })
    await userEvent.type(input, 'a@x.com{Enter}')
    await userEvent.type(input, 'b@x.com,')
    await userEvent.type(input, 'c@x.com;')
    await userEvent.type(input, 'd@x.com')
    await userEvent.click(screen.getByRole('button', { name: 'elsewhere' }))
    expect(addresses()).toBe('a@x.com|b@x.com|c@x.com|d@x.com')
    expect(input).toHaveValue('')
  })

  it('splits a pasted "Name <a@b>, c@d" list into chips with names', async () => {
    render(<Harness />)
    await userEvent.click(screen.getByRole('textbox', { name: '收件人' }))
    await userEvent.paste('Zhang San <zs@example.com>, li@example.org')
    expect(addresses()).toBe('zs@example.com|li@example.org')
    expect(screen.getByText('Zhang San')).toBeInTheDocument()
  })

  it('removes the last chip on Backspace in an empty input, but not while typing', async () => {
    render(<Harness initial={[{ name: null, address: 'a@x.com', valid: true }, { name: null, address: 'b@x.com', valid: true }]} />)
    const input = screen.getByRole('textbox', { name: '收件人' })
    await userEvent.type(input, 'q{Backspace}')
    expect(addresses()).toBe('a@x.com|b@x.com')
    await userEvent.keyboard('{Backspace}')
    expect(addresses()).toBe('a@x.com')
  })

  it('shows an invalid address as an error chip that can be edited or removed', async () => {
    render(<Harness />)
    const input = screen.getByRole('textbox', { name: '收件人' })
    await userEvent.type(input, 'nope{Enter}')
    expect(addresses()).toBe('!nope')
    const chip = screen.getByRole('button', { name: '地址无效：nope，点击修改' })
    expect(chip.parentElement).toHaveAttribute('data-invalid')
    expect(input).toHaveAttribute('aria-invalid', 'true')
    await userEvent.click(chip)
    expect(addresses()).toBe('')
    expect(input).toHaveValue('nope')
    await userEvent.type(input, '{Enter}')
    await userEvent.click(screen.getByRole('button', { name: '移除 nope' }))
    expect(addresses()).toBe('')
  })

  it('ignores duplicates', async () => {
    render(<Harness />)
    const input = screen.getByRole('textbox', { name: '收件人' })
    await userEvent.type(input, 'a@x.com{Enter}A@X.com{Enter}')
    expect(addresses()).toBe('a@x.com')
  })
})
