import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { TooltipProvider } from '@/components/ui/tooltip'
import { IconButton } from './icon-button'

const button = (disabled: boolean) => (
  <TooltipProvider>
    <IconButton label="回复" disabled={disabled}>x</IconButton>
  </TooltipProvider>
)

describe('IconButton tooltip', () => {
  it('shows the tooltip once a button that started disabled becomes enabled', async () => {
    const { rerender } = render(button(true))
    rerender(button(false))
    await userEvent.hover(screen.getByRole('button', { name: '回复' }))
    expect(await screen.findByText('回复', { selector: '[data-slot=tooltip-content]' })).toBeInTheDocument()
  })
})
