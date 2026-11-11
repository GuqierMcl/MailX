import { afterEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { QueryClientProvider } from '@tanstack/react-query'
import { render as baseRender, screen, waitFor, within } from '@/test/render'
import { testClient } from '@/features/accounts/test-fixtures'
import { Toaster } from '@/components/ui/toast'
import type { ContactAggregation } from '@/generated/accounts'
import { createQueryClient, contactKeys, mailKeys } from '@/lib/query-client'
import { ContactsPane } from './contacts-pane'

const rule = (address: string, name: string | null, overrides: Partial<ContactAggregation> = {}): ContactAggregation =>
  ({ address, name, messageCount: 5, latestReceivedAt: '2020-03-04T08:00:00.000Z', createdAt: '2026-10-01T00:00:00.000Z', ...overrides })

function setup(rules: ContactAggregation[]) {
  const client = testClient()
  let current = rules
  client.listContactAggregations.mockImplementation(async () => current)
  client.setContactAggregation.mockImplementation(async (address, aggregated) => {
    if (!aggregated) current = current.filter((item) => item.address !== address)
  })
  const queryClient = createQueryClient()
  const invalidate = vi.spyOn(queryClient, 'invalidateQueries')
  baseRender(<QueryClientProvider client={queryClient}><ContactsPane client={client} /><Toaster /></QueryClientProvider>)
  return { client, invalidate }
}

afterEach(() => vi.restoreAllMocks())

describe('contacts settings page', () => {
  it('lists each grouped contact with name, address and message count', async () => {
    setup([rule('wang@example.com', '王芳', { messageCount: 12 }), rule('news@example.com', null, { messageCount: 3, latestReceivedAt: null })])
    expect(await screen.findByText('邮件聚合')).toBeInTheDocument()
    expect(screen.getByText(/收成一行/)).toBeInTheDocument()
    const items = screen.getAllByRole('listitem')
    expect(items).toHaveLength(2)
    expect(within(items[0]).getByText('王芳')).toBeInTheDocument()
    expect(within(items[0]).getByText('wang@example.com')).toBeInTheDocument()
    expect(within(items[0]).getByText(/^12 封邮件 · 最近 /)).toBeInTheDocument()
    expect(within(items[1]).getByText('news@example.com')).toBeInTheDocument()
    expect(within(items[1]).getByText('3 封邮件')).toBeInTheDocument()
  })

  it('stops grouping a contact and refreshes the page and the lists', async () => {
    const { client, invalidate } = setup([rule('wang@example.com', '王芳'), rule('li@example.com', '李雷')])
    await userEvent.click(await screen.findByRole('button', { name: '取消聚合 王芳' }))
    expect(client.setContactAggregation).toHaveBeenCalledWith('wang@example.com', false)
    await waitFor(() => expect(screen.queryByText('王芳')).not.toBeInTheDocument())
    expect(screen.getByText('李雷')).toBeInTheDocument()
    expect(invalidate).toHaveBeenCalledWith({ queryKey: contactKeys.aggregations() })
    expect(invalidate).toHaveBeenCalledWith({ queryKey: mailKeys.emailLists() })
  })

  it('says so when stopping fails and keeps the contact', async () => {
    const { client } = setup([rule('wang@example.com', '王芳')])
    client.setContactAggregation.mockRejectedValue(new Error('io'))
    await userEvent.click(await screen.findByRole('button', { name: '取消聚合 王芳' }))
    expect((await screen.findAllByText('无法取消聚合')).length).toBeGreaterThan(0)
    expect(screen.getByText('王芳')).toBeInTheDocument()
  })

  it('explains how to add contacts when there are none', async () => {
    setup([])
    expect(await screen.findByText('还没有聚合的联系人')).toBeInTheDocument()
    expect(screen.getByText(/在邮件上点右键即可添加/)).toBeInTheDocument()
    expect(screen.queryByRole('listitem')).not.toBeInTheDocument()
  })

  it('shows a retryable error when the rules cannot be read', async () => {
    const client = testClient()
    client.listContactAggregations.mockRejectedValueOnce(new Error('io')).mockResolvedValue([rule('wang@example.com', '王芳')])
    baseRender(<QueryClientProvider client={createQueryClient()}><ContactsPane client={client} /></QueryClientProvider>)
    expect(await screen.findByText('无法读取聚合的联系人')).toBeInTheDocument()
    await userEvent.click(screen.getByRole('button', { name: '重试' }))
    expect(await screen.findByText('王芳')).toBeInTheDocument()
  })
})
