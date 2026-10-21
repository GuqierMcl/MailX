import { render, screen } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { testClient } from '@/features/accounts/test-fixtures'
import { withTitleBar } from '@/test/title-bar'
import { ThemeToggle } from './theme-toggle'

describe('ThemeToggle', () => {
  it('cycles system, light, dark and saves each step', async () => {
    const client = testClient()
    render(withTitleBar(<ThemeToggle client={client} />))
    await userEvent.click(await screen.findByRole('button', { name: '外观：跟随系统，点击切换到浅色' }))
    await userEvent.click(await screen.findByRole('button', { name: '外观：浅色，点击切换到深色' }))
    await userEvent.click(await screen.findByRole('button', { name: '外观：深色，点击切换到跟随系统' }))
    expect(client.updateSettings.mock.calls.map(([patch]) => patch.appearance?.theme)).toEqual(['light', 'dark', 'system'])
  })
})
