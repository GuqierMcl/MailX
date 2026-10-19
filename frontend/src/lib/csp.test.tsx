import { CSPProvider } from '@base-ui/react/csp-provider'
import { render } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'
import { ScrollArea } from '@/components/ui/scroll-area'

const injectedScrollbarStyles = () =>
  [...document.querySelectorAll('style')].filter((style) => style.textContent?.includes('base-ui-disable-scrollbar'))

afterEach(() => injectedScrollbarStyles().forEach((style) => style.remove()))

describe('scroll areas under the desktop CSP', () => {
  it('inject no <style> tag that style-src self would block', () => {
    render(
      <CSPProvider disableStyleElements>
        <ScrollArea>content</ScrollArea>
      </CSPProvider>,
    )
    expect(injectedScrollbarStyles()).toHaveLength(0)
  })

  it('would inject one without the provider, which is why main.tsx sets it', () => {
    render(<ScrollArea>content</ScrollArea>)
    expect(injectedScrollbarStyles().length).toBeGreaterThan(0)
  })
})
