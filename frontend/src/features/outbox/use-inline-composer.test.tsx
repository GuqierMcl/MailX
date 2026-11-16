import { act, renderHook } from '@testing-library/react'
import { QueryClientProvider } from '@tanstack/react-query'
import type { ReactNode } from 'react'
import { describe, expect, it, vi } from 'vitest'
import { testClient, exampleAccount } from '@/features/accounts/test-fixtures'
import type { ComposerController } from '@/features/composer/composer'
import { InlineDraftUnsaved, prepareMainWindowQuit } from '@/features/lifecycle/prepare-quit'
import { createQueryClient } from '@/lib/query-client'
import { toast } from '@/components/ui/toast'
import { useComposeActions } from './use-compose'
import { useInlineComposer } from './use-inline-composer'

const DRAFT = '11111111-1111-4111-8111-111111111111'

function controller(overrides: Partial<ComposerController> = {}): ComposerController {
  return {
    flush: vi.fn(async () => ({ ok: true })),
    isEmpty: () => true,
    hasChanges: () => false,
    restoreBaseline: vi.fn(async () => undefined),
    discard: vi.fn(async () => undefined),
    isReadOnly: () => false,
    isLoaded: () => true,
    ...overrides,
  }
}

function setup() {
  const queryClient = createQueryClient()
  const wrapper = ({ children }: { children: ReactNode }) => <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  const client = testClient([exampleAccount()])
  const hook = renderHook(() => useInlineComposer(client, { onShow: () => undefined, onContinue: () => undefined }), { wrapper })
  return { queryClient, hook, client }
}

/** The inline composer and the compose routing together, as the workbench wires them. */
function setupWithActions(mode: 'window' | 'inline') {
  const queryClient = createQueryClient()
  const wrapper = ({ children }: { children: ReactNode }) => <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  const accounts = [exampleAccount()]
  const client = testClient(accounts)
  const hook = renderHook(() => {
    const inline = useInlineComposer(client, { onShow: () => undefined, onContinue: () => undefined })
    return { inline, actions: useComposeActions(client, accounts, null, { mode, inline }) }
  }, { wrapper })
  return { hook, client }
}

describe('the inline composer on app quit', () => {
  it('writes the draft before the quit is confirmed, and keeps even a blank one', async () => {
    const { queryClient, hook } = setup()
    const editor = controller()
    act(() => hook.result.current.show(DRAFT, true))
    hook.result.current.controllerRef.current = editor
    await expect(prepareMainWindowQuit(queryClient, new AbortController().signal)).resolves.toBeUndefined()
    expect(editor.flush).toHaveBeenCalledTimes(1)
    // The quit may still be cancelled elsewhere, so nothing is deleted now.
    expect(editor.discard).not.toHaveBeenCalled()
    expect(hook.result.current.current?.draftId).toBe(DRAFT)
  })

  it('cancels the quit and keeps the editor when the write fails', async () => {
    const { queryClient, hook } = setup()
    const editor = controller({ flush: vi.fn(async () => ({ ok: false })), isEmpty: () => false })
    act(() => hook.result.current.show(DRAFT, false))
    hook.result.current.controllerRef.current = editor
    await expect(prepareMainWindowQuit(queryClient, new AbortController().signal)).rejects.toBeInstanceOf(InlineDraftUnsaved)
    expect(hook.result.current.current?.draftId).toBe(DRAFT)
  })

  it('has nothing to write without an inline composer', async () => {
    const { queryClient } = setup()
    await expect(prepareMainWindowQuit(queryClient, new AbortController().signal)).resolves.toBeUndefined()
  })

  it('stops taking part once unmounted', async () => {
    const { queryClient, hook } = setup()
    const editor = controller({ flush: vi.fn(async () => ({ ok: false })) })
    act(() => hook.result.current.show(DRAFT, false))
    hook.result.current.controllerRef.current = editor
    hook.unmount()
    await expect(prepareMainWindowQuit(queryClient, new AbortController().signal)).resolves.toBeUndefined()
  })
})

describe('leaving before the draft has loaded', () => {
  it('deletes a new draft quietly: nothing could have been typed', async () => {
    const { hook } = setup()
    const success = vi.spyOn(toast, 'success')
    const editor = controller({ isLoaded: () => false, isEmpty: () => false })
    act(() => hook.result.current.show(DRAFT, true))
    hook.result.current.controllerRef.current = editor
    await act(async () => { await expect(hook.result.current.leave()).resolves.toBe(true) })
    expect(editor.discard).toHaveBeenCalledTimes(1)
    expect(editor.flush).not.toHaveBeenCalled()
    expect(success).not.toHaveBeenCalled()
    expect(hook.result.current.current).toBeNull()
  })

  it('just closes an existing draft, without deleting it or a toast', async () => {
    const { hook } = setup()
    const success = vi.spyOn(toast, 'success')
    const editor = controller({ isLoaded: () => false, isEmpty: () => false })
    act(() => hook.result.current.show(DRAFT, false))
    hook.result.current.controllerRef.current = editor
    await act(async () => { await expect(hook.result.current.leave()).resolves.toBe(true) })
    expect(editor.discard).not.toHaveBeenCalled()
    expect(success).not.toHaveBeenCalled()
    expect(hook.result.current.current).toBeNull()
  })
})

describe('one draft, one editor', () => {
  it('opening the inline draft in a window writes it, opens the window with fresh, and closes inline', async () => {
    const { hook, client } = setupWithActions('window')
    const order: string[] = []
    const editor = controller({ isEmpty: () => false, flush: vi.fn(async () => { order.push('flush'); return { ok: true } }) })
    client.openComposer.mockImplementation(async () => { order.push('open') })
    act(() => hook.result.current.inline.show(DRAFT, true))
    hook.result.current.inline.controllerRef.current = editor
    await act(() => hook.result.current.actions.openDraft(DRAFT))
    expect(order).toEqual(['flush', 'open'])
    expect(client.openComposer).toHaveBeenCalledWith(DRAFT, true)
    expect(hook.result.current.inline.current).toBeNull()
    expect(editor.discard).not.toHaveBeenCalled()
  })

  it('keeps the inline draft when it cannot be written, and opens no window', async () => {
    const { hook, client } = setupWithActions('window')
    const editor = controller({ isEmpty: () => false, flush: vi.fn(async () => ({ ok: false })) })
    act(() => hook.result.current.inline.show(DRAFT, false))
    hook.result.current.inline.controllerRef.current = editor
    await act(() => hook.result.current.actions.openDraft(DRAFT))
    expect(client.openComposer).not.toHaveBeenCalled()
    expect(hook.result.current.inline.current?.draftId).toBe(DRAFT)
  })

  it('inline, a draft already open there is kept as it is', async () => {
    const { hook, client } = setupWithActions('inline')
    const editor = controller({ isEmpty: () => false })
    act(() => hook.result.current.inline.show(DRAFT, true))
    const key = hook.result.current.inline.current?.key
    hook.result.current.inline.controllerRef.current = editor
    await act(() => hook.result.current.actions.openDraft(DRAFT))
    expect(hook.result.current.inline.current?.key).toBe(key)
    expect(editor.flush).not.toHaveBeenCalled()
    expect(client.focusComposer).not.toHaveBeenCalled()
  })
})
