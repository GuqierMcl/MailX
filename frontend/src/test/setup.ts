import '@testing-library/jest-dom/vitest'
import { cleanup } from '@testing-library/react'
import { afterEach, vi } from 'vitest'
import { i18n } from '@/i18n'

afterEach(() => {
  cleanup()
  void i18n.changeLanguage('zh-CN')
  localStorage.removeItem('mailx.language')
  localStorage.removeItem('mailx.mock.settings')
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
