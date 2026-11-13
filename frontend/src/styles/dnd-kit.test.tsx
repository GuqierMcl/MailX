import { StyleInjector } from '@dnd-kit/dom'
import { describe, expect, it, vi } from 'vitest'
import { render } from '@/test/render'
import { SidebarProvider } from '@/components/ui/sidebar'
import { exampleAccount } from '@/features/accounts/test-fixtures'
import { SortableAccountList } from '@/features/accounts/sortable-account-list'
import bundled from './dnd-kit.css?raw'

const normalize = (css: string) => css.replace(/\/\*[\s\S]*?\*\//g, ' ').replace(/\s+/g, ' ').trim()

/** CSS text each dnd-kit plugin hands to StyleInjector when the list mounts. */
function injectedRules() {
  const register = vi.spyOn(StyleInjector.prototype, 'register')
  const { unmount } = render(
    <SidebarProvider>
      <SortableAccountList accounts={[exampleAccount(), exampleAccount({ id: 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb' })]}
        activeId={null} syncStates={{}} disabled={false} reorderDisabled={false} onSelect={() => {}} onReorder={async () => {}} />
    </SidebarProvider>,
  )
  const rules = [...new Set(register.mock.calls.map(([css]) => normalize(css)))]
  unmount()
  return rules
}

describe('dnd-kit styles under the desktop CSP', () => {
  it('ships every rule dnd-kit would inject as a <style> tag that style-src self blocks', () => {
    const rules = injectedRules()
    const css = normalize(bundled)
    // The feedback rules are copied verbatim. The cursor and selection rules
    // are scoped to an active drag, so they are matched by their declarations.
    const scoped = new Map([
      ['* { cursor: grabbing !important; }', 'html:has([data-dnd-dragging]) * { cursor: grabbing !important; }'],
      ['* { user-select: none !important; -webkit-user-select: none !important; }',
        'html:has([data-dnd-dragging]) * { user-select: none !important; -webkit-user-select: none !important; }'],
    ])
    const feedback = rules.filter((rule) => !scoped.has(rule))
    // A dnd-kit upgrade that adds, removes or edits an injected rule fails
    // here: update styles/dnd-kit.css to match the library's new rules.
    expect(feedback).toHaveLength(1)
    expect(feedback[0]).toContain('[data-dnd-dragging]')
    expect(css).toContain(feedback[0])
    expect(rules.filter((rule) => scoped.has(rule)).sort()).toEqual([...scoped.keys()].sort())
    for (const replacement of scoped.values()) expect(css).toContain(replacement)
  })
})
