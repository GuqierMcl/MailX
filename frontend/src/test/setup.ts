import '@testing-library/jest-dom/vitest'
import { cleanup } from '@testing-library/react'
import { afterEach, vi } from 'vitest'
import { i18n } from '@/i18n'
import { testViewport } from './viewport'

afterEach(() => {
  cleanup()
  void i18n.changeLanguage('zh-CN')
  localStorage.removeItem('mailx.language')
  localStorage.removeItem('mailx.mock.settings')
  testViewport.height = 10000
})

Object.defineProperty(window, 'matchMedia', {
  writable: true,
  value: vi.fn().mockImplementation((query: string) => ({
    matches: false, media: query, onchange: null,
    addListener: vi.fn(), removeListener: vi.fn(), addEventListener: vi.fn(), removeEventListener: vi.fn(), dispatchEvent: vi.fn(),
  })),
})

class TestResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}
vi.stubGlobal('ResizeObserver', TestResizeObserver)
Element.prototype.scrollIntoView = vi.fn()
Element.prototype.getAnimations = vi.fn(() => [])

// jsdom has no layout. Scroll-area viewports report a large box so virtualized lists
// render every row of a small dataset; other elements keep reporting nothing.
const isViewport = (element: Element) => element.getAttribute('data-slot') === 'scroll-area-viewport'
for (const [property, size] of [['offsetHeight', 'height'], ['clientHeight', 'height'], ['offsetWidth', 'width'], ['clientWidth', 'width']] as const) {
  Object.defineProperty(HTMLElement.prototype, property, {
    configurable: true,
    get(this: HTMLElement) {
      return isViewport(this) ? testViewport[size] : 0
    },
  })
}
const baseRect = Element.prototype.getBoundingClientRect
Element.prototype.getBoundingClientRect = function (this: Element) {
  if (!isViewport(this)) return baseRect.call(this)
  const { width, height } = testViewport
  return { x: 0, y: 0, top: 0, left: 0, right: width, bottom: height, width, height, toJSON: () => ({}) }
}
