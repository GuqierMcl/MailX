import type { ReactNode } from 'react'
import { QueryClientProvider } from '@tanstack/react-query'
import { render as baseRender, type RenderOptions } from '@testing-library/react'
import { createQueryClient } from '@/lib/query-client'

// Test-only module, never hot reloaded: re-export Testing Library alongside the wrapped render.
// eslint-disable-next-line react-refresh/only-export-components
export * from '@testing-library/react'

/** Testing Library's render with a fresh query cache per test, as the app root provides. */
export function render(ui: ReactNode, options?: Omit<RenderOptions, 'wrapper'>) {
  const client = createQueryClient()
  return baseRender(ui, {
    wrapper: ({ children }) => <QueryClientProvider client={client}>{children}</QueryClientProvider>,
    ...options,
  })
}
