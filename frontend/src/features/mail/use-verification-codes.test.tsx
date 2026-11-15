import { useEffect } from 'react'
import { act, fireEvent, render, screen, waitFor } from '@/test/render'
import { describe, expect, it, vi } from 'vitest'
import { testClient } from '@/features/accounts/test-fixtures'
import { Toaster } from '@/components/ui/toast'
import { t } from '@/i18n'
import type { EmailSummary, VerificationCodeCopyOutcome } from '@/generated/accounts'
import { codeRequestLimiter, useVerificationCodes, type VerificationCodes, type VerificationTarget } from './use-verification-codes'

function deferred<T>() {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((complete) => { resolve = complete })
  return { promise, resolve }
}

const target: VerificationTarget = { emailId: 'otp', bodyState: 'missing' }
const targets = [target]
const result: VerificationCodeCopyOutcome = { emailId: 'otp', accountId: 'a', cleanup: 'disabled', deletion: null }
function Harness({ client, onHidden, visible = targets, reader = null, capture }: {
  client: ReturnType<typeof testClient>; onHidden: (email: { id: string }) => void; visible?: VerificationTarget[]
  reader?: EmailSummary | null; capture?: (controller: VerificationCodes) => void
}) {
  const codes = useVerificationCodes(client, reader, 'source', onHidden)
  const request = codes.requestVisible
  useEffect(() => { request(visible) }, [request, visible])
  useEffect(() => { capture?.(codes) }, [capture, codes])
  return <>
    <span data-testid="code-state">{codes.status('otp')?.state ?? 'pending'}</span>
    <button disabled={codes.busy('otp')} onClick={() => void codes.copy('otp', 'recognition')}>copy</button>
    <Toaster />
  </>
}

describe('verification code requests', () => {
  it('runs at most two requests and skips a queued row removed before it starts', async () => {
    const run = codeRequestLimiter()
    const one = deferred<number>(), two = deferred<number>()
    const first = run(() => one.promise, new AbortController().signal)
    const second = run(() => two.promise, new AbortController().signal)
    const canceled = new AbortController()
    const queued = vi.fn(async () => 3)
    const third = run(queued, canceled.signal).catch((error: unknown) => error)
    expect(queued).not.toHaveBeenCalled()
    canceled.abort()
    one.resolve(1)
    expect(await first).toBe(1)
    expect(await third).toBeInstanceOf(DOMException)
    expect(queued).not.toHaveBeenCalled()
    two.resolve(2)
    expect(await second).toBe(2)
  })

  it('recognizes visible rows independently of notification settings and reruns when the body arrives', async () => {
    const client = testClient()
    const onHidden = vi.fn()
    const { rerender } = render(<Harness client={client} onHidden={onHidden} />)
    await waitFor(() => expect(client.getVerificationCode).toHaveBeenCalledWith('otp', true))
    expect(client.getSettings).not.toHaveBeenCalled()
    const ready: VerificationTarget[] = [{ ...target, bodyState: 'ready' }]
    rerender(<Harness client={client} onHidden={onHidden} visible={ready} />)
    await waitFor(() => expect(client.getVerificationCode).toHaveBeenCalledTimes(2))
  })
})

describe('shared copy outcomes', () => {
  it('applies a background notification cleanup without copying a second time', async () => {
    const client = testClient(), onHidden = vi.fn()
    let copied: (outcome: VerificationCodeCopyOutcome) => void = () => undefined
    client.onVerificationCodeCopied.mockImplementation((listener) => { copied = listener; return () => undefined })
    render(<Harness client={client} onHidden={onHidden} />)
    await waitFor(() => expect(client.onVerificationCodeCopied).toHaveBeenCalled())
    act(() => copied({ ...result, cleanup: 'queued', deletion: { batchId: 'background', undoUntil: new Date(Date.now() + 5000).toISOString(), permanent: false, skippedIds: [] } }))
    expect(onHidden).toHaveBeenCalledWith({ id: 'otp' })
    expect(client.copyVerificationCode).not.toHaveBeenCalled()
    expect(client.deleteEmails).not.toHaveBeenCalled()
  })
  it('waits for clipboard success, prevents duplicate clicks, then offers deletion undo', async () => {
    const client = testClient()
    const onHidden = vi.fn()
    const pending = deferred<VerificationCodeCopyOutcome>()
    client.copyVerificationCode.mockReturnValue(pending.promise)
    let codes: VerificationCodes | undefined
    const capture = (next: VerificationCodes) => { codes = next }
    render(<Harness client={client} onHidden={onHidden} capture={capture} />)
    await waitFor(() => expect(codes).toBeDefined())
    act(() => { void codes!.copy('otp', 'recognition'); void codes!.copy('otp', 'recognition') })
    expect(client.copyVerificationCode).toHaveBeenCalledTimes(1)
    expect(onHidden).not.toHaveBeenCalled()
    await act(async () => pending.resolve({ ...result, cleanup: 'queued', deletion: { batchId: 'batch', undoUntil: new Date(Date.now() + 5000).toISOString(), permanent: false, skippedIds: [] } }))
    await waitFor(() => expect(onHidden).toHaveBeenCalledWith({ id: 'otp' }))
    fireEvent.click(await screen.findByRole('button', { name: t('mail:undo') }))
    await waitFor(() => expect(client.undoVerificationCodeDelete).toHaveBeenCalledWith('batch'))
    expect(client.deleteEmails).not.toHaveBeenCalled()
  })

  it.each(['unavailable', 'failed'] as const)('reports copied but retained for %s cleanup', async (cleanup) => {
    const client = testClient(), onHidden = vi.fn()
    client.copyVerificationCode.mockResolvedValue({ ...result, cleanup })
    render(<Harness client={client} onHidden={onHidden} />)
    fireEvent.click(screen.getByRole('button', { name: 'copy' }))
    expect((await screen.findAllByText(t(cleanup === 'failed' ? 'mail:verificationCleanupFailed' : 'mail:verificationCopiedKept')))[0]).toBeInTheDocument()
    expect(onHidden).not.toHaveBeenCalled()
  })

  it('leaves the email alone when copying fails', async () => {
    const client = testClient(), onHidden = vi.fn()
    client.copyVerificationCode.mockRejectedValue(new Error('clipboard unavailable'))
    render(<Harness client={client} onHidden={onHidden} />)
    fireEvent.click(screen.getByRole('button', { name: 'copy' }))
    expect((await screen.findAllByText(t('mail:verificationCopyFailed')))[0]).toBeInTheDocument()
    expect(onHidden).not.toHaveBeenCalled()
    expect(client.deleteEmails).not.toHaveBeenCalled()
  })
})
