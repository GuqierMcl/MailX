import { describe, expect, it } from 'vitest'
import { composerDraftId, composerSearch, windowRole } from './composer-window'

const ID = '3f2b8c1e-9a4d-4e6f-8b1a-2c3d4e5f6a7b'

describe('composer window detection', () => {
  it('reads the draft id from a canonical query', () => {
    expect(composerDraftId(`?composer=${ID}`)).toBe(ID)
    expect(composerDraftId(`?x=1&composer=${ID}`)).toBe(ID)
    expect(windowRole(`?composer=${ID}`)).toEqual({ kind: 'composer', draftId: ID })
    expect(composerDraftId(composerSearch(ID))).toBe(ID)
  })

  it('treats a missing or malformed id as the main window', () => {
    for (const search of ['', '?', '?composer=', '?composer=x', `?composer=${ID.toUpperCase()}`, `?composer=${ID}x`, `?composer=${ID.replaceAll('-', '')}`, `?composer[]=${ID}`]) {
      expect(composerDraftId(search)).toBeNull()
      expect(windowRole(search)).toEqual({ kind: 'main' })
    }
  })
})
