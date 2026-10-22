import { useCallback, useEffect, useRef, useState } from 'react'
import type { Account } from '@/generated/accounts'
import { clientFailure, type ClientFailure, type MailxClient } from '@/lib/mailx-client'

/**
 * Loads one account through `getAccount`. `version` advances on every load
 * and local replacement, so forms keyed on it start from the fresh record.
 */
export function useAccount(client: MailxClient, accountId: string | null) {
  const [account, setAccount] = useState<Account | null>(null)
  const [error, setError] = useState<ClientFailure | null>(null)
  const [version, setVersion] = useState(0)
  const sequence = useRef(0)

  const load = useCallback(async () => {
    const current = ++sequence.current
    setAccount(null)
    setError(null)
    if (!accountId) return
    try {
      const detail = await client.getAccount(accountId)
      if (current !== sequence.current) return
      setAccount(detail)
      setVersion((value) => value + 1)
    } catch (cause) {
      if (current === sequence.current) setError(clientFailure(cause))
    }
  }, [client, accountId])

  useEffect(() => {
    // Load outside the effect body; the sequence counter drops stale responses.
    void Promise.resolve().then(load)
  }, [load])

  /** Adopts a record the caller just saved, without another round trip. */
  const replace = useCallback((next: Account) => {
    sequence.current++
    setAccount(next)
    setError(null)
    setVersion((value) => value + 1)
  }, [])

  return { account, error, version, reload: load, replace }
}
