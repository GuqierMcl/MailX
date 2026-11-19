import { describe, expect, it } from 'vitest'
import { exampleFolderSyncState, exampleSyncState } from './test-fixtures'
import { failureVisible, failingSince, QUIET_FOR_MS, visibleFolderFailure, visibleSyncFailure } from './sync-quiet'

const NOW = Date.parse('2026-10-18T08:00:00.000Z')
const ago = (ms: number) => new Date(NOW - ms).toISOString()

const network = { code: 'network' as const, message: '无法连接到邮件服务器。', detail: null, willRetry: true }
const authentication = { code: 'authentication' as const, message: '服务器拒绝了登录。', detail: null, willRetry: false }

describe('quiet retryable failures', () => {
  it('hides a retryable failure until it has lasted the whole quiet window', () => {
    expect(failureVisible(ago(QUIET_FOR_MS - 1), NOW)).toBe(false)
    expect(failureVisible(ago(QUIET_FOR_MS), NOW)).toBe(true)
    expect(failureVisible(ago(QUIET_FOR_MS * 4), NOW)).toBe(true)
  })

  it('has no age to judge a failure without a start, so it stays hidden', () => {
    expect(failureVisible(null, NOW)).toBe(false)
  })

  it('shows a failure that needs a person at once', () => {
    const sync = exampleSyncState({ phase: 'failed', failure: authentication, failureSince: ago(1_000) })
    expect(visibleSyncFailure(sync, NOW)).toEqual(authentication)
  })

  it('hides a young network failure and shows an old one', () => {
    const young = exampleSyncState({ phase: 'failed', failure: network, failureSince: ago(5_000) })
    expect(visibleSyncFailure(young, NOW)).toBeNull()
    const old = exampleSyncState({ phase: 'failed', failure: network, failureSince: ago(QUIET_FOR_MS) })
    expect(visibleSyncFailure(old, NOW)).toEqual(network)
  })

  it('measures from the oldest failing folder when the account itself has no clock', () => {
    const sync = exampleSyncState({
      failure: network,
      failureSince: null,
      folders: [
        exampleFolderSyncState('inbox', { phase: 'failed', failure: network, failureSince: ago(QUIET_FOR_MS) }),
        exampleFolderSyncState('sent', { phase: 'failed', failure: network, failureSince: ago(1_000) }),
      ],
    })
    expect(failingSince(sync)).toBe(ago(QUIET_FOR_MS))
    expect(visibleSyncFailure(sync, NOW)).toEqual(network)
  })

  it('takes the earlier of the account and folder clocks', () => {
    const sync = exampleSyncState({
      failure: network,
      failureSince: ago(1_000),
      folders: [exampleFolderSyncState('inbox', { phase: 'failed', failure: network, failureSince: ago(QUIET_FOR_MS) })],
    })
    expect(failingSince(sync)).toBe(ago(QUIET_FOR_MS))
  })

  it('judges a folder by its own clock', () => {
    const young = exampleFolderSyncState('inbox', { phase: 'failed', failure: network, failureSince: ago(1_000) })
    expect(visibleFolderFailure(young, NOW)).toBeNull()
    const old = exampleFolderSyncState('inbox', { phase: 'failed', failure: network, failureSince: ago(QUIET_FOR_MS) })
    expect(visibleFolderFailure(old, NOW)).toEqual(network)
    const stopped = exampleFolderSyncState('inbox', { phase: 'failed', failure: authentication, failureSince: ago(1_000) })
    expect(visibleFolderFailure(stopped, NOW)).toEqual(authentication)
  })

  it('keeps a protocol failure visible at once: it is not weather', () => {
    const sync = exampleSyncState({
      phase: 'failed',
      failure: { code: 'protocol', message: '服务器返回了无法理解的结果。', detail: null, willRetry: true },
      failureSince: ago(1_000),
    })
    expect(visibleSyncFailure(sync, NOW)?.code).toBe('protocol')
  })
})
