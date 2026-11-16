import { fireEvent, render, screen, waitFor } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { useState } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/tooltip'
import type { Recipient } from './address-parse'
import { RecipientField, type RecipientRow } from './recipient-field'

function Harness({ initial = [], row, onMove }: { initial?: Recipient[]; row?: RecipientRow; onMove?: (index: number, to: RecipientRow) => void }) {
  const [value, setValue] = useState<Recipient[]>(initial)
  return (
    <TooltipProvider delay={0}>
      <RecipientField label="收件人" value={value} onChange={setValue} row={row} onMove={onMove} />
      <output aria-label="addresses">{value.map((item) => `${item.valid ? '' : '!'}${item.address}`).join('|')}</output>
      <button type="button">elsewhere</button>
    </TooltipProvider>
  )
}

const addresses = () => screen.getByLabelText('addresses').textContent

describe('recipient chips', () => {
  it('commits on Enter, comma, semicolon and blur', async () => {
    render(<Harness />)
    const input = screen.getByRole('combobox', { name: '收件人' })
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
    await userEvent.click(screen.getByRole('combobox', { name: '收件人' }))
    await userEvent.paste('Zhang San <zs@example.com>, li@example.org')
    expect(addresses()).toBe('zs@example.com|li@example.org')
    expect(screen.getByText('Zhang San')).toBeInTheDocument()
  })

  it('removes the last chip on Backspace in an empty input, but not while typing', async () => {
    render(<Harness initial={[{ name: null, address: 'a@x.com', valid: true }, { name: null, address: 'b@x.com', valid: true }]} />)
    const input = screen.getByRole('combobox', { name: '收件人' })
    await userEvent.type(input, 'q{Backspace}')
    expect(addresses()).toBe('a@x.com|b@x.com')
    await userEvent.keyboard('{Backspace}')
    expect(addresses()).toBe('a@x.com')
  })

  it('shows an invalid address as an error chip that can be edited or removed', async () => {
    render(<Harness />)
    const input = screen.getByRole('combobox', { name: '收件人' })
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
    const input = screen.getByRole('combobox', { name: '收件人' })
    await userEvent.type(input, 'a@x.com{Enter}A@X.com{Enter}')
    expect(addresses()).toBe('a@x.com')
  })
})

describe('chip names, tooltip and menu', () => {
  const zhang = { name: '张三', address: 'zs@example.com', valid: true }

  it('shows the name, with the full name and address in a tooltip', async () => {
    render(<Harness initial={[zhang, { name: null, address: 'li@example.org', valid: true }]} />)
    expect(screen.getByRole('button', { name: '张三' })).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'li@example.org' })).toBeInTheDocument()
    await userEvent.hover(screen.getByRole('button', { name: '张三' }))
    expect(await screen.findByText('张三 <zs@example.com>')).toBeInTheDocument()
  })

  it('offers moving to the other two rows, copying the address and removing it', async () => {
    const onMove = vi.fn()
    const writeText = vi.fn(async () => undefined)
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true })
    render(<Harness initial={[zhang]} row="cc" onMove={onMove} />)
    fireEvent.contextMenu(screen.getByRole('button', { name: '张三' }))
    const menu = await screen.findByRole('menu')
    const items = Array.from(menu.querySelectorAll('[role=menuitem]')).map((item) => item.textContent)
    expect(items).toEqual(['移到收件人', '移到密送', '复制地址', '移除'])
    await userEvent.click(screen.getByRole('menuitem', { name: '移到密送' }))
    expect(onMove).toHaveBeenCalledWith(0, 'bcc')

    fireEvent.contextMenu(screen.getByRole('button', { name: '张三' }))
    await userEvent.click(await screen.findByRole('menuitem', { name: '复制地址' }))
    expect(writeText).toHaveBeenCalledWith('zs@example.com')

    fireEvent.contextMenu(screen.getByRole('button', { name: '张三' }))
    await userEvent.click(await screen.findByRole('menuitem', { name: '移除' }))
    await waitFor(() => expect(addresses()).toBe(''))
  })

  it('has no move items outside the composer rows', async () => {
    render(<Harness initial={[zhang]} />)
    fireEvent.contextMenu(screen.getByRole('button', { name: '张三' }))
    const menu = await screen.findByRole('menu')
    expect(Array.from(menu.querySelectorAll('[role=menuitem]')).map((item) => item.textContent)).toEqual(['复制地址', '移除'])
  })
})
