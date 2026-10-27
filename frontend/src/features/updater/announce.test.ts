import { describe, expect, it } from 'vitest'
import { announcements, shouldAnnounce } from './announce'
import type { UpdateSnapshot } from './types'

const idle: UpdateSnapshot = { canInstall: true, phase: 'idle' }
const found: UpdateSnapshot = { canInstall: true, phase: 'available', version: '0.2.0', current: '0.1.0', notes: null, date: null }
const ready: UpdateSnapshot = { canInstall: true, phase: 'ready', version: '0.2.0' }

describe('what the user hears about', () => {
  it('announces a version once, not on every refetch of the same state', () => {
    expect(announcements(idle, found, false)).toEqual([{ kind: 'available', version: '0.2.0', manual: false }])
    expect(announcements(found, found, false)).toEqual([])
    expect(announcements({ ...found, phase: 'checking' }, found, false)).toEqual([
      { kind: 'available', version: '0.2.0', manual: false },
    ])
  })

  it('keeps a manual check out of the toasts', () => {
    const [notice] = announcements({ ...found, phase: 'checking' }, found, true)
    expect(shouldAnnounce(notice, new Set(), false)).toBe(false)
    const [automatic] = announcements(idle, found, false)
    expect(shouldAnnounce(automatic, new Set(['0.2.0']), false)).toBe(false)
    expect(shouldAnnounce(automatic, new Set(), false)).toBe(true)
  })

  it('announces a finished download only while the modal is closed', () => {
    const [notice] = announcements(found, ready, false)
    expect(notice).toEqual({ kind: 'ready', version: '0.2.0' })
    expect(shouldAnnounce(notice, new Set(), true)).toBe(false)
    expect(shouldAnnounce(notice, new Set(), false)).toBe(true)
  })

  it('says nothing about idle, checking or failed states', () => {
    expect(announcements(found, { canInstall: true, phase: 'failed', stage: 'download', version: '0.2.0' }, false)).toEqual([])
    expect(announcements(null, { canInstall: true, phase: 'checking' }, false)).toEqual([])
  })
})
