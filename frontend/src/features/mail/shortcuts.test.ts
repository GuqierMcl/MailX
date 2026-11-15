import { afterEach, describe, expect, it } from 'vitest'
import { matchShortcut, SHORTCUT_GROUPS, SHORTCUTS } from './shortcuts'

/** A keydown sent from `target` (the body by default), as the window listener sees it. */
function press(key: string, init: KeyboardEventInit = {}, target: Element = document.body) {
  let result: ReturnType<typeof matchShortcut> = null
  const listener = (event: KeyboardEvent) => { result = matchShortcut(event) }
  window.addEventListener('keydown', listener)
  target.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...init }))
  window.removeEventListener('keydown', listener)
  return result
}

function mount(html: string) {
  const host = document.createElement('div')
  host.innerHTML = html
  document.body.append(host)
  return host
}

afterEach(() => { document.body.innerHTML = '' })

describe('shortcut definitions', () => {
  it('give every single-key shortcut keys to match and every shortcut a group', () => {
    const groups = new Set(SHORTCUT_GROUPS.map((group) => group.id))
    for (const item of SHORTCUTS) {
      expect(groups.has(item.group)).toBe(true)
      expect(item.keys.length).toBeGreaterThan(0)
      expect(Boolean(item.match?.length)).toBe(item.singleKey)
    }
    const matched = SHORTCUTS.flatMap((item) => item.match ?? [])
    expect(new Set(matched).size).toBe(matched.length)
  })
})

describe('matchShortcut', () => {
  it('maps keys by event.key, letters in either case', () => {
    expect(press('j')).toBe('next')
    expect(press('J')).toBe('next')
    expect(press('ArrowDown')).toBe('next')
    expect(press('k')).toBe('previous')
    expect(press('Enter')).toBe('open')
    expect(press('e')).toBe('archive')
    expect(press('Backspace')).toBe('delete')
    expect(press('Delete')).toBe('delete')
    expect(press('a')).toBe('replyAll')
    expect(press('x')).toBeNull()
  })

  it('accepts Shift only for symbols', () => {
    expect(press('#', { shiftKey: true })).toBe('delete')
    expect(press('?', { shiftKey: true })).toBe('help')
    expect(press('/', { shiftKey: true })).toBe('search')
    expect(press('E', { shiftKey: true })).toBeNull()
    expect(press('ArrowDown', { shiftKey: true })).toBeNull()
    expect(press('Enter', { shiftKey: true })).toBeNull()
  })

  it('ignores any other modifier', () => {
    for (const modifier of ['metaKey', 'ctrlKey', 'altKey'] as const) {
      expect(press('e', { [modifier]: true })).toBeNull()
      expect(press('r', { [modifier]: true })).toBeNull()
    }
  })

  it('ignores IME composition', () => {
    expect(press('e', { isComposing: true })).toBeNull()
    expect(press('e', { keyCode: 229 })).toBeNull()
  })

  it('ignores text entry of every kind', () => {
    const host = mount(`
      <input /><textarea></textarea><select><option>x</option></select>
      <div contenteditable="true"><span id="inner">x</span></div>
      <div contenteditable=""></div>
      <div role="textbox"></div><div role="combobox"></div><div role="searchbox"></div><div role="spinbutton"></div>
    `)
    for (const element of host.querySelectorAll('input, textarea, select, #inner, [contenteditable=""], [role]')) {
      expect(press('e', {}, element)).toBeNull()
    }
    expect(press('e', {}, mount('<div contenteditable="false"><span>x</span></div>').querySelector('span')!)).toBe('archive')
  })

  it('ignores keys while a dialog, menu or popup is open', () => {
    for (const html of [
      '<div role="dialog" data-open></div>',
      '<div role="alertdialog" data-open></div>',
      '<div role="menu"></div>',
      '<div role="listbox"></div>',
    ]) {
      const layer = mount(html)
      expect(press('e')).toBeNull()
      layer.remove()
    }
    // A dialog that has finished closing no longer counts.
    mount('<div role="dialog" data-closed></div>')
    expect(press('e')).toBe('archive')
  })

  it('leaves Enter to focused controls and arrows to controls that use them', () => {
    const host = mount('<button>x</button><div role="tablist"><div role="tab">t</div></div><div role="separator" tabindex="0"></div>')
    expect(press('Enter', {}, host.querySelector('button')!)).toBeNull()
    expect(press('e', {}, host.querySelector('button')!)).toBe('archive')
    expect(press('ArrowDown', {}, host.querySelector('[role="tab"]')!)).toBeNull()
    expect(press('ArrowUp', {}, host.querySelector('[role="separator"]')!)).toBeNull()
    expect(press('ArrowDown', {}, host.querySelector('button')!)).toBe('next')
  })

  it('ignores a key press already handled', () => {
    const listener = (event: KeyboardEvent) => event.preventDefault()
    document.body.addEventListener('keydown', listener)
    expect(press('e')).toBeNull()
    document.body.removeEventListener('keydown', listener)
  })
})
