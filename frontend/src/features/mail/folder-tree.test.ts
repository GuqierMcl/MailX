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
    countsKnown: true,
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

  it('propagates unknown counts and ignores stale counts on a Noselect container', () => {
    const [root] = buildFolderTree([
      folder('container', { selectable: false, countsKnown: false, unreadCount: 99 }),
      folder('child', { parentId: 'container', countsKnown: false }),
    ])
    expect(root.unreadWithChildren).toBe(0)
    expect(root.countsKnown).toBe(false)
    expect(buildFolderTree([folder('container', { selectable: false, countsKnown: false })])[0].countsKnown).toBe(true)
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

describe('junk mail in unread totals', () => {
  it('never adds to a parent or an account number, but is remembered for a dot', () => {
    const tree = buildFolderTree([
      folder('inbox', { role: 'inbox', unreadCount: 2 }),
      folder('junk', { role: 'junk', unreadCount: 7 }),
      folder('work', { unreadCount: 1 }),
      folder('spam-in-work', { parentId: 'work', role: 'junk', unreadCount: 4 }),
    ])
    const byId = Object.fromEntries(tree.map((node) => [node.folder.id, node]))
    expect(tree.reduce((total, node) => total + node.unreadWithChildren, 0)).toBe(3)
    expect(byId.junk.unreadWithChildren).toBe(0)
    expect(byId.junk.junkUnread).toBe(true)
    // A parent keeps only its own and its ordinary children's mail, and points at junk below.
    expect(byId.work.unreadWithChildren).toBe(1)
    expect(byId.work.junkUnread).toBe(true)
    expect(byId.inbox.junkUnread).toBe(false)
  })

  it('has no dot for a junk folder with nothing unread or one that is only a container', () => {
    const tree = buildFolderTree([
      folder('junk', { role: 'junk', unreadCount: 0 }),
      folder('box', { role: 'junk', selectable: false, unreadCount: 9 }),
    ])
    expect(tree.every((node) => !node.junkUnread && node.unreadWithChildren === 0)).toBe(true)
  })
})
