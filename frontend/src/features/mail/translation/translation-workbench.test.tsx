import { render, screen, waitFor, within } from '@/test/render'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'
import { createMockClient } from '@/lib/mock-client'
import { withTitleBar } from '@/test/title-bar'
import { Workbench } from '../workbench'

async function openFirstMessage(client = createMockClient()) {
  render(withTitleBar(<Workbench client={client} />))
  const list = await screen.findByRole('list', { name: '邮件列表' })
  await userEvent.click(await within(list).findByText('第三季度产品路线图评审'))
  expect(await screen.findByRole('heading', { level: 2, name: '第三季度产品路线图评审' })).toBeInTheDocument()
  return client
}

describe('translation in the workbench', () => {
  it('sends "去设置" from the toolbar popover to the translation page of the settings', async () => {
    const client = createMockClient()
    const settings = (await client.getSettings()).settings.translation
    await client.updateSettings({ translation: { ...settings, provider: null } })
    await openFirstMessage(client)
    await userEvent.click(await screen.findByRole('button', { name: '翻译' }))
    await userEvent.click(await screen.findByRole('button', { name: '去设置' }))
    const dialog = await screen.findByRole('dialog', { name: '设置' })
    expect(await within(dialog).findByRole('combobox', { name: '服务' })).toHaveTextContent('不使用翻译')
    expect(within(dialog).getByRole('heading', { level: 2, name: '翻译' })).toBeInTheDocument()
  })

  it('offers the palette commands for the open message and flips between translation and original', async () => {
    const client = createMockClient()
    await client.setTranslationKey('deepl', 'key')
    const settings = (await client.getSettings()).settings.translation
    await client.updateSettings({ translation: { ...settings, provider: 'deepl', consentedProvider: 'deepl' } })
    await openFirstMessage(client)
    await userEvent.click(screen.getByRole('button', { name: '搜索邮件' }))
    await userEvent.type(await screen.findByRole('combobox'), '翻译')
    await userEvent.click(await screen.findByRole('option', { name: '翻译这封邮件' }))
    await waitFor(() => expect(screen.getByRole('button', { name: '翻译' })).toHaveAttribute('aria-pressed', 'true'), { timeout: 3000 })
    expect(screen.getByRole('status', { name: '邮件翻译' })).toHaveTextContent('已从中文翻译')
    await userEvent.click(screen.getByRole('button', { name: '搜索邮件' }))
    await userEvent.type(await screen.findByRole('combobox'), '原文')
    await userEvent.click(await screen.findByRole('option', { name: '显示原文' }))
    await waitFor(() => expect(screen.getByRole('button', { name: '翻译' })).toHaveAttribute('aria-pressed', 'false'))
  })

  it('translates out of the box: hint, consent, translated, without opening the settings', async () => {
    await openFirstMessage()
    expect(screen.queryByRole('dialog', { name: '设置' })).not.toBeInTheDocument()
    await userEvent.click(await screen.findByRole('button', { name: '翻译' }))
    const dialog = await screen.findByRole('alertdialog')
    expect(dialog).toHaveTextContent('把邮件内容发送给Microsoft 翻译？')
    expect(dialog).toHaveTextContent('邮件的主题和正文会发送给Microsoft 翻译，')
    await userEvent.click(within(dialog).getByRole('button', { name: '翻译' }))
    await waitFor(() => expect(screen.getByRole('button', { name: '翻译' })).toHaveAttribute('aria-pressed', 'true'), { timeout: 3000 })
    expect(screen.queryByRole('dialog', { name: '设置' })).not.toBeInTheDocument()
  })

  it('without a service the palette command goes to the settings instead', async () => {
    const client = createMockClient()
    const settings = (await client.getSettings()).settings.translation
    await client.updateSettings({ translation: { ...settings, provider: null } })
    await openFirstMessage(client)
    await userEvent.click(screen.getByRole('button', { name: '搜索邮件' }))
    await userEvent.type(await screen.findByRole('combobox'), '翻译')
    await userEvent.click(await screen.findByRole('option', { name: '翻译这封邮件' }))
    const dialog = await screen.findByRole('dialog', { name: '设置' })
    expect(await within(dialog).findByRole('combobox', { name: '服务' })).toBeInTheDocument()
  })
})
