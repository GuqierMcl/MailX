import { createEvent, fireEvent, render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { testClient } from '@/features/accounts/test-fixtures'
import { UpdaterProvider } from '@/features/updater/updater-provider'
import { SettingsDialog } from './settings-dialog'

const REPOSITORY = 'https://github.com/GuqierMcl/MailX'
const clipboard = Object.getOwnPropertyDescriptor(navigator, 'clipboard')

function renderAbout(client = testClient()) {
  render(
    <UpdaterProvider client={client}>
      <SettingsDialog client={client} target={{ section: 'about' }} onClose={() => undefined} />
    </UpdaterProvider>
  )
  return { client, project: screen.getByRole('region', { name: '项目与反馈' }) }
}

afterEach(() => {
  vi.restoreAllMocks()
  if (clipboard) Object.defineProperty(navigator, 'clipboard', clipboard)
  else Reflect.deleteProperty(navigator, 'clipboard')
})

describe('About project links', () => {
  it('opens the repository first, both issue templates and release history through the desktop client', () => {
    const { client, project } = renderAbout()
    const links = within(project).getAllByRole('link')
    const destinations = [REPOSITORY, `${REPOSITORY}/issues/new?template=bug_report.md`, `${REPOSITORY}/issues/new?template=feature_request.md`, `${REPOSITORY}/releases`]
    expect(links.map((link) => link.getAttribute('href'))).toEqual(destinations)

    for (const [index, link] of links.entries()) {
      const click = createEvent.click(link)
      fireEvent(link, click)
      expect(click.defaultPrevented).toBe(true)
      expect(client.openExternalLink).toHaveBeenNthCalledWith(index + 1, destinations[index])
    }
    expect(client.openExternalLink).toHaveBeenCalledTimes(4)
  })

  it('keeps keyboard and middle-click activation in the system browser', async () => {
    const { client, project } = renderAbout()
    const link = within(project).getByRole('link', { name: 'GitHub' })
    link.focus()
    await userEvent.keyboard('{Enter}')
    expect(client.openExternalLink).toHaveBeenCalledWith(REPOSITORY)

    const middleClick = new MouseEvent('auxclick', { bubbles: true, cancelable: true, button: 1 })
    fireEvent(link, middleClick)
    expect(middleClick.defaultPrevented).toBe(true)
    expect(client.openExternalLink).toHaveBeenCalledTimes(2)
  })

  it('offers the failed link for copying without revealing the host error', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined)
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } })
    const client = testClient()
    client.openExternalLink.mockRejectedValue(new Error('/private/host/path: cannot open browser'))
    const { project } = renderAbout(client)
    await userEvent.click(within(project).getByRole('link', { name: '反馈问题' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('无法打开链接')
    expect(screen.queryByText(/\/private\/host\/path/)).not.toBeInTheDocument()

    await userEvent.click(await screen.findByRole('button', { name: '复制链接' }))
    await waitFor(() => expect(writeText).toHaveBeenCalledWith(`${REPOSITORY}/issues/new?template=bug_report.md`))
    expect(await screen.findByText('已复制链接')).toBeInTheDocument()
  })

  it('reports clipboard failure and leaves the public URL available', async () => {
    const writeText = vi.fn().mockRejectedValue(new Error('private clipboard service details'))
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: { writeText } })
    const client = testClient()
    client.openExternalLink.mockRejectedValue(new Error('browser unavailable'))
    const { project } = renderAbout(client)
    await userEvent.click(within(project).getByRole('link', { name: '功能建议' }))
    await userEvent.click(await screen.findByRole('button', { name: '复制链接' }))

    expect(await screen.findByText('无法复制链接')).toBeInTheDocument()
    expect(screen.getByText(`${REPOSITORY}/issues/new?template=feature_request.md`)).toBeInTheDocument()
    expect(screen.queryByText(/private clipboard service details/)).not.toBeInTheDocument()
  })
})
