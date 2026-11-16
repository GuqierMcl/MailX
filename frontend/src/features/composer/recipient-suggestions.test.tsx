import { act, fireEvent, render, screen, waitFor } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { useState } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { TooltipProvider } from '@/components/ui/tooltip'
import type { ContactSuggestion } from '@/generated/accounts'
import type { Recipient } from './address-parse'
import { RecipientField } from './recipient-field'

const zhang = { name: '张三', address: 'zhang@example.com' }
const zoe = { name: 'Zoe Li', address: 'zoe@corp.example' }
const bare = { name: null, address: 'zed@example.org' }

function Harness({ suggest, exclude, initial = [] }: { suggest: (query: string, limit?: number) => Promise<ContactSuggestion[]>; exclude?: Set<string>; initial?: Recipient[] }) {
  const [value, setValue] = useState<Recipient[]>(initial)
  const taken = new Set([...(exclude ?? []), ...value.map((item) => item.address.toLowerCase())])
  return (
    <TooltipProvider>
      <RecipientField label="收件人" value={value} onChange={setValue} suggest={suggest} exclude={taken} />
      <output aria-label="addresses">{value.map((item) => `${item.name ?? ''}<${item.address}>`).join('|')}</output>
      <button type="button">elsewhere</button>
    </TooltipProvider>
  )
}

function setup(results: ContactSuggestion[] = [zhang, zoe, bare], props: Partial<Parameters<typeof Harness>[0]> = {}) {
  const suggest = vi.fn(async () => results)
  render(<Harness suggest={suggest} {...props} />)
  const input = screen.getByRole('combobox', { name: '收件人' })
  return { suggest, input }
}

const addresses = () => screen.getByLabelText('addresses').textContent
const options = () => screen.queryAllByRole('option')
const listbox = () => screen.findByRole('listbox')

describe('recipient suggestions', () => {
  it('asks once typing pauses and lists names over addresses, highlighting the first', async () => {
    const { suggest, input } = setup()
    await userEvent.type(input, 'z')
    await listbox()
    expect(suggest).toHaveBeenCalledTimes(1)
    expect(suggest).toHaveBeenCalledWith('z', expect.any(Number))
    expect(options().map((option) => option.textContent)).toEqual(['张三zhang@example.com', 'Zoe Lizoe@corp.example', 'zed@example.org'])
    expect(input).toHaveAttribute('aria-expanded', 'true')
    expect(input).toHaveAttribute('aria-activedescendant', options()[0].id)
    expect(options()[0]).toHaveAttribute('aria-selected', 'true')
    // The matched start of the address (or a name word) is bold.
    expect(options()[1].querySelectorAll('strong')[0]).toHaveTextContent('Z')
  })

  it('debounces: quick typing makes one request', async () => {
    const { suggest, input } = setup()
    await userEvent.type(input, 'zoe', { delay: null })
    await listbox()
    expect(suggest).toHaveBeenCalledTimes(1)
    expect(suggest).toHaveBeenCalledWith('zoe', expect.any(Number))
  })

  it('moves with the arrow keys and picks with Enter', async () => {
    const { input } = setup()
    await userEvent.type(input, 'z')
    await listbox()
    await userEvent.keyboard('{ArrowDown}{ArrowDown}{ArrowDown}')
    expect(options()[0]).toHaveAttribute('aria-selected', 'true')
    await userEvent.keyboard('{ArrowUp}')
    expect(options()[2]).toHaveAttribute('aria-selected', 'true')
    await userEvent.keyboard('{ArrowUp}{Enter}')
    expect(addresses()).toBe('Zoe Li<zoe@corp.example>')
    expect(input).toHaveValue('')
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
    expect(input).toHaveFocus()
  })

  it('picks with Tab and keeps focus in the field', async () => {
    const { input } = setup()
    await userEvent.type(input, 'z')
    await listbox()
    await userEvent.keyboard('{Tab}')
    expect(addresses()).toBe('张三<zhang@example.com>')
    expect(input).toHaveFocus()
  })

  it('picks with a click without the field losing focus first', async () => {
    const { input } = setup()
    await userEvent.type(input, 'z')
    await listbox()
    const press = fireEvent.pointerDown(options()[2])
    expect(press).toBe(false)
    await userEvent.click(options()[2])
    expect(addresses()).toBe('<zed@example.org>')
    expect(input).toHaveFocus()
  })

  it('Esc closes only the list: the text stays and the key goes no further', async () => {
    const { input } = setup()
    const outside = vi.fn()
    await userEvent.type(input, 'z')
    await listbox()
    window.addEventListener('keydown', outside)
    await userEvent.keyboard('{Escape}')
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
    expect(input).toHaveValue('z')
    expect(outside).not.toHaveBeenCalled()
    // With no list open, Esc is not swallowed.
    await userEvent.keyboard('{Escape}')
    expect(outside).toHaveBeenCalledTimes(1)
    window.removeEventListener('keydown', outside)
  })

  it('shows only the newest request’s answer', async () => {
    let finishFirst: (value: ContactSuggestion[]) => void = () => undefined
    const suggest = vi.fn()
      .mockImplementationOnce(() => new Promise<ContactSuggestion[]>((resolve) => { finishFirst = resolve }))
      .mockImplementationOnce(async () => [zoe])
    render(<Harness suggest={suggest} />)
    const input = screen.getByRole('combobox', { name: '收件人' })
    await userEvent.type(input, 'z')
    await waitFor(() => expect(suggest).toHaveBeenCalledTimes(1))
    await userEvent.type(input, 'o')
    await waitFor(() => expect(options().map((option) => option.textContent)).toEqual(['Zoe Lizoe@corp.example']))
    await act(async () => finishFirst([zhang, bare]))
    expect(options().map((option) => option.textContent)).toEqual(['Zoe Lizoe@corp.example'])
  })

  it('leaves out addresses already in any row, ignoring case', async () => {
    setup([zhang, zoe, bare], { exclude: new Set(['zoe@corp.example']), initial: [{ name: null, address: 'ZED@example.org', valid: true }] })
    await userEvent.type(screen.getByRole('combobox', { name: '收件人' }), 'z')
    await listbox()
    expect(options().map((option) => option.textContent)).toEqual(['张三zhang@example.com'])
  })

  it('shows nothing (not "no results") when nothing matches, and never asks for blank text', async () => {
    const { suggest, input } = setup([])
    await userEvent.type(input, ' ')
    await new Promise((resolve) => setTimeout(resolve, 150))
    expect(suggest).not.toHaveBeenCalled()
    await userEvent.type(input, 'q')
    await waitFor(() => expect(suggest).toHaveBeenCalled())
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
    expect(input).toHaveAttribute('aria-expanded', 'false')
  })

  it('does not ask while an IME is composing, and Enter during composition never picks', async () => {
    const { suggest, input } = setup()
    await userEvent.type(input, 'z')
    await listbox()
    suggest.mockClear()
    fireEvent.compositionStart(input)
    fireEvent.change(input, { target: { value: 'zh' } })
    fireEvent.keyDown(input, { key: 'Enter', isComposing: true })
    fireEvent.keyDown(input, { key: 'Enter', keyCode: 229 })
    await new Promise((resolve) => setTimeout(resolve, 150))
    expect(suggest).not.toHaveBeenCalled()
    expect(addresses()).toBe('')
    fireEvent.compositionEnd(input)
    await waitFor(() => expect(suggest).toHaveBeenCalledWith('zh', expect.any(Number)))
  })

  it('still turns typed text into a chip at a comma, closing the list', async () => {
    const { input } = setup()
    await userEvent.type(input, 'zed@x.com')
    await listbox()
    await userEvent.type(input, ',')
    expect(addresses()).toBe('<zed@x.com>')
    expect(input).toHaveValue('')
    await waitFor(() => expect(screen.queryByRole('listbox')).not.toBeInTheDocument())
  })

  it('leaves Cmd/Ctrl+Enter to the send shortcut: the typed text becomes a chip, not the suggestion', async () => {
    const { input } = setup()
    await userEvent.type(input, 'zed@x.com')
    await listbox()
    const event = fireEvent.keyDown(input, { key: 'Enter', metaKey: true })
    expect(event).toBe(true)
    expect(addresses()).toBe('<zed@x.com>')
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
  })

  it('closes when the field loses focus', async () => {
    const { input } = setup()
    await userEvent.type(input, 'q')
    await listbox()
    await userEvent.click(screen.getByRole('button', { name: 'elsewhere' }))
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
  })
})
