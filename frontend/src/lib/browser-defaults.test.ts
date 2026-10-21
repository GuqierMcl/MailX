import { afterEach, describe, expect, it, vi } from 'vitest'
import { installBrowserDefaultGuards, isBrowserShortcut } from './browser-defaults'

let remove: (() => void) | undefined
afterEach(() => {
  remove?.()
  remove = undefined
  document.body.innerHTML = ''
})

function key(init: KeyboardEventInit) {
  return new KeyboardEvent('keydown', { bubbles: true, cancelable: true, ...init })
}

function contextMenuOn(element: Element) {
  const event = new MouseEvent('contextmenu', { bubbles: true, cancelable: true })
  element.dispatchEvent(event)
  return event.defaultPrevented
}

describe('context menu', () => {
  it('is suppressed on ordinary content but kept on text fields', () => {
    remove = installBrowserDefaultGuards()
    document.body.innerHTML = `
      <p id="text">text</p><button id="button">b</button><input id="box" type="checkbox">
      <input id="input"><input id="search" type="search"><textarea id="area"></textarea>
      <div id="editable" contenteditable="true"><span id="inner">x</span></div>
      <div id="off" contenteditable="false"><span id="off-inner">y</span></div>`
    const prevented = (id: string) => contextMenuOn(document.getElementById(id)!)
    for (const id of ['text', 'button', 'box', 'off', 'off-inner']) expect(prevented(id), id).toBe(true)
    for (const id of ['input', 'search', 'area', 'editable', 'inner']) expect(prevented(id), id).toBe(false)
  })
})

describe('browser shortcuts', () => {
  const blocked: [string, KeyboardEventInit][] = [
    ['F5', { code: 'F5' }],
    ['Ctrl+F5', { code: 'F5', ctrlKey: true }],
    ['Ctrl+R', { code: 'KeyR', ctrlKey: true }],
    ['Cmd+R', { code: 'KeyR', metaKey: true }],
    ['Ctrl+Shift+R', { code: 'KeyR', ctrlKey: true, shiftKey: true }],
    ['Ctrl+P', { code: 'KeyP', ctrlKey: true }],
    ['Cmd+P', { code: 'KeyP', metaKey: true }],
    ['Ctrl+S', { code: 'KeyS', ctrlKey: true }],
    ['Ctrl+U', { code: 'KeyU', ctrlKey: true }],
    ['Ctrl+F', { code: 'KeyF', ctrlKey: true }],
    ['F3', { code: 'F3' }],
    ['Ctrl+G', { code: 'KeyG', ctrlKey: true }],
    ['F12', { code: 'F12' }],
    ['Ctrl+Shift+I', { code: 'KeyI', ctrlKey: true, shiftKey: true }],
    ['Cmd+Option+I', { code: 'KeyI', metaKey: true, altKey: true }],
    ['Cmd+Option+U', { code: 'KeyU', metaKey: true, altKey: true }],
    ['Alt+Left', { code: 'ArrowLeft', altKey: true }],
    ['Alt+Right', { code: 'ArrowRight', altKey: true }],
    ['Cmd+[', { code: 'BracketLeft', metaKey: true }],
    ['Cmd+]', { code: 'BracketRight', metaKey: true }],
  ]
  it.each(blocked)('blocks %s', (_name, init) => {
    expect(isBrowserShortcut({ ctrlKey: false, metaKey: false, altKey: false, shiftKey: false, ...init } as KeyboardEvent)).toBe(true)
    remove = installBrowserDefaultGuards()
    const event = key(init)
    document.body.dispatchEvent(event)
    expect(event.defaultPrevented).toBe(true)
  })

  const allowed: [string, KeyboardEventInit][] = [
    ['plain letters', { code: 'KeyR' }],
    ['Tab', { code: 'Tab' }],
    ['Shift+Tab', { code: 'Tab', shiftKey: true }],
    ['Ctrl+C', { code: 'KeyC', ctrlKey: true }],
    ['Cmd+C', { code: 'KeyC', metaKey: true }],
    ['Ctrl+V', { code: 'KeyV', ctrlKey: true }],
    ['Ctrl+X', { code: 'KeyX', ctrlKey: true }],
    ['Ctrl+A', { code: 'KeyA', ctrlKey: true }],
    ['Ctrl+Z', { code: 'KeyZ', ctrlKey: true }],
    ['Cmd+Q', { code: 'KeyQ', metaKey: true }],
    ['Escape', { code: 'Escape' }],
    ['Alt+Tab', { code: 'Tab', altKey: true }],
    ['Ctrl+Alt+R', { code: 'KeyR', ctrlKey: true, altKey: true }],
    ['arrow keys', { code: 'ArrowLeft' }],
    ['Ctrl+Left', { code: 'ArrowLeft', ctrlKey: true }],
  ]
  it.each(allowed)('leaves %s alone', (_name, init) => {
    remove = installBrowserDefaultGuards()
    const event = key(init)
    document.body.dispatchEvent(event)
    expect(event.defaultPrevented).toBe(false)
  })

  it('never stops propagation, so the app can bind the same keys', () => {
    remove = installBrowserDefaultGuards()
    const seen = vi.fn()
    document.body.addEventListener('keydown', seen)
    document.body.dispatchEvent(key({ code: 'KeyF', ctrlKey: true }))
    expect(seen).toHaveBeenCalledTimes(1)
  })

  it('removes its listeners again', () => {
    installBrowserDefaultGuards()()
    const event = key({ code: 'F5' })
    document.body.dispatchEvent(event)
    expect(event.defaultPrevented).toBe(false)
    expect(contextMenuOn(document.body)).toBe(false)
  })
})
