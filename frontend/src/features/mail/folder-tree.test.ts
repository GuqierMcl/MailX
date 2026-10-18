import { describe, expect, it } from 'vitest'
import type { Folder } from '@/generated/accounts'
import { buildFolderTree, loadCollapsed, saveCollapsed } from './folder-tree'

function folder(id: string, overrides: Partial<Folder> = {}): Folder {
  return {
    id,
    accountId: 'account',
    parentId: null,
    name: id,
    role: 'other',
    selectable: true,
    unreadCount: 0,
    totalCount: 0,
    ...overrides,
  }
}

describe('buildFolderTree', () => {
  it('puts system folders first in a fixed order, then user folders by name with numeric order', () => {
    const tree = buildFolderTree([
      folder('trash', { role: 'trash' }),
      folder('archive', { role: 'archive' }),
      folder('Archive 2', { name: 'Archive 2' }),
      folder('inbox', { role: 'inbox' }),
      folder('Archive 10', { name: 'Archive 10' }),
      folder('sent', { role: 'sent' }),
    ])
    expect(tree.map((node) => node.folder.id)).toEqual(['inbox', 'sent', 'archive', 'trash', 'Archive 2', 'Archive 10'])
  })

  it('nests children and rolls unread counts up to every ancestor', () => {
    const [root] = buildFolderTree([
      folder('clients', { unreadCount: 1 }),
      folder('east', { parentId: 'clients', unreadCount: 2 }),
      folder('shanghai', { parentId: 'east', unreadCount: 3 }),
    ])
    expect(root.folder.id).toBe('clients')
    expect(root.unreadWithChildren).toBe(6)
    expect(root.children[0].unreadWithChildren).toBe(5)
    expect(root.children[0].children[0].folder.id).toBe('shanghai')
  })

  it('keeps a folder whose parent is missing at the top level', () => {
    expect(buildFolderTree([folder('orphan', { parentId: 'gone' })]).map((node) => node.folder.id)).toEqual(['orphan'])
  })
})

describe('collapsed state', () => {
  it('round-trips through storage and ignores corrupt values', () => {
    saveCollapsed(new Set(['a', 'b']))
    expect([...loadCollapsed()]).toEqual(['a', 'b'])
    localStorage.setItem('mailx.sidebar.collapsed', '{not json')
    expect(loadCollapsed().size).toBe(0)
    localStorage.setItem('mailx.sidebar.collapsed', '[1, "c"]')
    expect([...loadCollapsed()]).toEqual(['c'])
    localStorage.clear()
  })
})
