import { StrictMode } from 'react'
import { act, fireEvent, render, screen, waitFor } from '@/test/render'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { exampleAccount, testClient } from '@/features/accounts/test-fixtures'
import type { Draft } from '@/generated/accounts'

function PreviewEditor({ value }: { value: string }) {
  return <textarea aria-label="正文" value={value} readOnly />
}

function draft(): Draft {
  return {
    id: 'loading-draft', accountId: exampleAccount().id, kind: 'new', sourceEmailId: null,
    to: [], cc: [], bcc: [], subject: 'Original', bodyHtml: '<p>Saved body</p>', bodyText: 'Saved body',
    inReplyTo: null, references: null, createdAt: '2026-10-25T00:00:00Z', updatedAt: '2026-10-25T00:00:00Z', revision: 1,
  }
}

beforeEach(() => vi.resetModules())
afterEach(() => vi.doUnmock('./mail-body-editor'))

describe('editor loading', () => {
  it('starts the editor while draft IPC is still pending, even under StrictMode', async () => {
    let finishImport!: (module: { default: typeof PreviewEditor }) => void
    const importStarted = vi.fn(() => new Promise<{ default: typeof PreviewEditor }>((resolve) => { finishImport = resolve }))
    vi.doMock('./mail-body-editor', importStarted)
    const { Composer } = await import('./composer')
    const client = testClient([exampleAccount()])
    let finishDraft!: (value: Draft) => void
    const pendingDraft = new Promise<Draft>((resolve) => { finishDraft = resolve })
    client.getDraft.mockReturnValue(pendingDraft)
    render(<StrictMode><Composer client={client} draftId="loading-draft" accounts={[exampleAccount()]} onSent={() => {}} onDiscarded={() => {}} /></StrictMode>)
    await waitFor(() => expect(importStarted).toHaveBeenCalledTimes(1))
    expect(screen.getByRole('status', { name: '正在打开草稿' })).toBeInTheDocument()
    await act(async () => { finishImport({ default: PreviewEditor }) })
    expect(screen.queryByRole('textbox', { name: '正文' })).not.toBeInTheDocument()
    await act(async () => { finishDraft(draft()) })
    expect(await screen.findByRole('textbox', { name: '正文' })).toHaveValue('<p>Saved body</p>')
    expect(screen.queryByText('正在打开编辑器…')).not.toBeInTheDocument()
    expect(client.updateDraft).not.toHaveBeenCalled()
  })

  it('does not import at module load, and renders a preloaded editor without a loading frame', async () => {
    const importStarted = vi.fn(() => ({ default: PreviewEditor }))
    vi.doMock('./mail-body-editor', importStarted)
    const { preloadMailBodyEditor, useMailBodyEditor } = await import('./mail-body-editor-loader')
    expect(importStarted).not.toHaveBeenCalled()
    const first = preloadMailBodyEditor()
    expect(preloadMailBodyEditor()).toBe(first)
    await first
    function Surface() {
      const { Component } = useMailBodyEditor()
      return Component ? <Component value="Ready" onChange={() => {}} /> : <p>Loading</p>
    }
    render(<Surface />)
    expect(screen.getByRole('textbox', { name: '正文' })).toHaveValue('Ready')
    expect(screen.queryByText('Loading')).not.toBeInTheDocument()
    expect(importStarted).toHaveBeenCalledTimes(1)
  })

  it('retries a failed import without reloading the draft or losing edits', async () => {
    vi.doMock('./mail-body-editor', () => { throw new Error('Temporary module request failure') })
    const { Composer } = await import('./composer')
    const client = testClient([exampleAccount()])
    client.getDraft.mockResolvedValue(draft())
    render(<Composer client={client} draftId="loading-draft" accounts={[exampleAccount()]} onSent={() => {}} onDiscarded={() => {}} />)
    expect(await screen.findByRole('alert')).toHaveTextContent('编辑器加载失败')
    fireEvent.change(screen.getByRole('textbox', { name: '主题' }), { target: { value: 'Keep my edit' } })
    vi.doMock('./mail-body-editor', () => ({ default: PreviewEditor }))
    fireEvent.click(screen.getByRole('button', { name: '重新加载编辑器' }))
    expect(await screen.findByRole('textbox', { name: '正文' })).toHaveValue('<p>Saved body</p>')
    expect(screen.getByRole('textbox', { name: '主题' })).toHaveValue('Keep my edit')
    expect(client.getDraft).toHaveBeenCalledTimes(1)
  })
})
