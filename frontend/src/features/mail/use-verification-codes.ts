import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useQueries, useQueryClient } from '@tanstack/react-query'
import type { EmailSummary, VerificationCodeCopyOutcome, VerificationCodeStatus } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'
import { mailKeys } from '@/lib/query-client'
import { errorText } from '@/i18n/errors'
import { t } from '@/i18n'
import { toast } from '@/components/ui/toast'

const CODE_KEYS = ['verification-codes'] as const
const COPIED_MS = 2000

export type VerificationTarget = { emailId: string; bodyState: EmailSummary['bodyState'] }
export type ReadyVerificationCode = { code: string; recognitionId: string }

export function readyVerificationCode(status: VerificationCodeStatus | undefined): ReadyVerificationCode | null {
  return status?.state === 'ready' && status.code && status.recognitionId
    ? { code: status.code, recognitionId: status.recognitionId } : null
}

export type VerificationCodes = {
  status: (emailId: string) => VerificationCodeStatus | undefined
  requestVisible: (targets: VerificationTarget[]) => void
  copy: (emailId: string, recognitionId: string) => Promise<void>
  busy: (emailId: string) => boolean
  copied: (emailId: string, recognitionId: string) => boolean
}

/** Queued requests whose rows disappeared are discarded before IPC starts. */
export function codeRequestLimiter(limit = 2) {
  let active = 0
  const waiting: (() => void)[] = []
  const pump = () => {
    while (active < limit && waiting.length > 0) waiting.shift()!()
  }
  return <T,>(work: () => Promise<T>, signal: AbortSignal): Promise<T> => new Promise((resolve, reject) => {
    waiting.push(() => {
      if (signal.aborted) {
        reject(new DOMException('Aborted', 'AbortError'))
        return
      }
      active += 1
      void Promise.resolve().then(() => {
        if (signal.aborted) throw new DOMException('Aborted', 'AbortError')
        return work()
      }).then(resolve, reject).finally(() => { active -= 1; pump() })
    })
    pump()
  })
}

/** The list, current reader and background notification results share one UI controller. */
export function useVerificationCodes(client: MailxClient, reader: EmailSummary | null, sourceIdentity: string, onHidden: (email: { id: string }) => void): VerificationCodes {
  const queryClient = useQueryClient()
  const [visible, setVisible] = useState<VerificationTarget[]>([])
  const limit = useMemo(() => codeRequestLimiter(), [])
  const busyRef = useRef(new Set<string>())
  const [busyIds, setBusyIds] = useState(new Set<string>())
  const [copiedIds, setCopiedIds] = useState(new Set<string>())
  const timers = useRef(new Map<string, ReturnType<typeof setTimeout>>())
  useEffect(() => {
    const pending = timers.current
    return () => { pending.forEach(clearTimeout) }
  }, [])
  const requestVisible = useCallback((next: VerificationTarget[]) => {
    setVisible((current) => JSON.stringify(current) === JSON.stringify(next) ? current : next)
  }, [])
  const targets = useMemo(() => {
    const wanted = new Map(visible.map((target) => [target.emailId, { ...target, allowFetch: true }]))
    if (reader && !reader.localOnly) wanted.set(reader.id, {
      emailId: reader.id, bodyState: reader.bodyState,
      // The ordinary reader already downloads a missing body. A list request
      // may fetch independently, but opening alone doesn't duplicate that work.
      allowFetch: wanted.get(reader.id)?.allowFetch ?? false,
    })
    return [...wanted.values()]
  }, [visible, reader])
  const queries = useQueries({
    queries: targets.map((target) => ({
      queryKey: [...CODE_KEYS, target.emailId, sourceIdentity, target.bodyState, target.allowFetch],
      queryFn: ({ signal }: { signal: AbortSignal }) => limit(() => client.getVerificationCode(target.emailId, target.allowFetch), signal),
      staleTime: 30_000,
      gcTime: 5 * 60_000,
      retry: false,
      refetchOnWindowFocus: true,
    })),
  })
  const statuses = new Map(targets.map((target, i) => [target.emailId, queries[i].data]))

  const refresh = useCallback((ids?: string[]) => {
    const relevant = ids?.length ? ids : undefined
    void queryClient.invalidateQueries({ queryKey: mailKeys.emailLists() })
    void queryClient.invalidateQueries({ queryKey: mailKeys.tree() })
    void queryClient.invalidateQueries({ queryKey: mailKeys.conversations() })
    void queryClient.invalidateQueries({ queryKey: mailKeys.all, predicate: (query) => query.queryKey[1] === 'email' && (!relevant || relevant.includes(String(query.queryKey[2]))) })
    void queryClient.invalidateQueries({ queryKey: CODE_KEYS, predicate: (query) => !relevant || relevant.includes(String(query.queryKey[1])) })
  }, [queryClient])
  const undo = useCallback(async (batchId: string) => {
    try {
      await client.undoVerificationCodeDelete(batchId)
      refresh()
    } catch (error) {
      toast.error(t('mail:undoDeleteFailed'), { description: errorText(error) })
    }
  }, [client, refresh])
  const finish = useCallback((outcome: VerificationCodeCopyOutcome, announce: boolean) => {
    if (outcome.cleanup === 'queued' && outcome.deletion) {
      onHidden({ id: outcome.emailId })
      refresh([outcome.emailId])
    }
    if (!announce) return
    if (outcome.cleanup === 'queued' && outcome.deletion) {
      const deletion = outcome.deletion
      const duration = Date.parse(deletion.undoUntil) - Date.now()
      toast.info(t('mail:verificationCopiedAndTrashed'), {
        timeout: Number.isFinite(duration) ? Math.max(0, Math.min(duration, 60_000)) : 0,
        action: { label: t('mail:undo'), onClick: () => void undo(deletion.batchId) },
      })
    } else if (outcome.cleanup === 'unavailable') toast.info(t('mail:verificationCopiedKept'))
    else if (outcome.cleanup === 'failed') toast.error(t('mail:verificationCleanupFailed'))
    else toast.info(t('mail:verificationCopied'))
  }, [onHidden, refresh, undo])
  useEffect(() => client.onVerificationCodeChanged(({ emailIds }) => refresh(emailIds)), [client, refresh])
  useEffect(() => client.onVerificationCodeCopied((outcome) => {
    finish(outcome, document.visibilityState === 'visible' && document.hasFocus())
  }), [client, finish])

  const copy = useCallback(async (emailId: string, recognitionId: string) => {
    if (busyRef.current.has(emailId)) return
    busyRef.current.add(emailId)
    setBusyIds(new Set(busyRef.current))
    try {
      const outcome = await client.copyVerificationCode(emailId, recognitionId)
      finish(outcome, true)
      const key = `${emailId}:${recognitionId}`
      setCopiedIds((current) => new Set(current).add(key))
      const previous = timers.current.get(key)
      if (previous) clearTimeout(previous)
      timers.current.set(key, setTimeout(() => {
        timers.current.delete(key)
        setCopiedIds((current) => { const next = new Set(current); next.delete(key); return next })
      }, COPIED_MS))
    } catch (error) {
      toast.error(t('mail:verificationCopyFailed'), { description: errorText(error) })
      void queryClient.invalidateQueries({ queryKey: [...CODE_KEYS, emailId] })
    } finally {
      busyRef.current.delete(emailId)
      setBusyIds(new Set(busyRef.current))
    }
  }, [client, finish, queryClient])
  return {
    status: (emailId) => {
      const status = statuses.get(emailId)
      return status?.emailId === emailId ? status : undefined
    }, requestVisible, copy,
    busy: (emailId) => busyIds.has(emailId),
    copied: (emailId, recognitionId) => copiedIds.has(`${emailId}:${recognitionId}`),
  }
}
