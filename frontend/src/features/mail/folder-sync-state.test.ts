import { describe, expect, it } from 'vitest'
import type { Folder } from '@/generated/accounts'
import { exampleAccount, exampleFolderSyncState, exampleSyncState } from '@/features/accounts/test-fixtures'
import { acceptsSyncState, folderSyncStatus } from './folder-sync-state'

const account = exampleAccount()
const sent: Folder = { id: 'sent', accountId: account.id, name: 'Sent', role: 'sent', parentId: null, selectable: true, countsKnown: true, unreadCount: 0, totalCount: 0 }

describe('folder sync state snapshots', () => {
  it('rejects another source and snapshots from older tasks or sequences', () => {
    const previous = exampleSyncState({ generation: 5, sequence: 8 })
    expect(acceptsSyncState(account, previous, exampleSyncState({ mailboxSourceId: 'old-source', generation: 99, sequence: 99 }))).toBe(false)
    expect(acceptsSyncState(account, previous, exampleSyncState({ mailboxSourceId: null, generation: 99, sequence: 99 }))).toBe(false)
    expect(acceptsSyncState(account, previous, exampleSyncState({ generation: 4, sequence: 99 }))).toBe(false)
    expect(acceptsSyncState(account, previous, exampleSyncState({ generation: 5, sequence: 8 }))).toBe(false)
    expect(acceptsSyncState(account, previous, exampleSyncState({ generation: 6, sequence: 1 }))).toBe(true)
    expect(acceptsSyncState(account, previous, exampleSyncState({ generation: 5, sequence: 9 }))).toBe(true)
  })

  it('does not apply a different folder failure to an already completed folder', () => {
    const state = exampleSyncState({
      phase: 'failed', failure: { code: 'protocol', message: '部分目录失败', detail: null, willRetry: true },
      folders: [exampleFolderSyncState('inbox', { phase: 'failed' }), exampleFolderSyncState('sent')],
    })
    expect(folderSyncStatus([sent], { [account.id]: state }, true)).toEqual({ phase: 'idle', failure: null, windowCapped: true })
  })

  it('shows an account failure when a folder could not start', () => {
    const state = exampleSyncState({
      phase: 'failed', failure: { code: 'authentication', message: '请检查账号设置。', detail: null, willRetry: false },
      folders: [exampleFolderSyncState('sent', { phase: 'waiting' })],
    })
    expect(folderSyncStatus([sent], { [account.id]: state }, false)).toEqual({ phase: 'failed', failure: '邮箱服务器拒绝了账号登录，请检查密码或授权码。', windowCapped: false })
  })

  it('shows a completed empty scope for an account without an inbox', () => {
    expect(folderSyncStatus([], { [account.id]: exampleSyncState() }, false).phase).toBe('idle')
  })
})
