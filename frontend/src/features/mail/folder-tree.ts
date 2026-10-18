import type { Folder, FolderRole } from '@/generated/accounts'

export type FolderNode = {
  folder: Folder
  children: FolderNode[]
  /** Unread count of this folder and every descendant, shown when it is collapsed. */
  unreadWithChildren: number
}

const ROLE_ORDER: FolderRole[] = ['inbox', 'drafts', 'sent', 'archive', 'junk', 'trash', 'other']

const collator = new Intl.Collator('zh-CN', { numeric: true })

function compareFolders(a: Folder, b: Folder) {
  return ROLE_ORDER.indexOf(a.role) - ROLE_ORDER.indexOf(b.role) || collator.compare(a.name, b.name)
}

/**
 * Builds one account's folder tree: system folders first in a fixed order,
 * then user folders by name. A folder whose parent is missing is shown at the
 * top level rather than dropped.
 */
export function buildFolderTree(folders: Folder[]): FolderNode[] {
  const ids = new Set(folders.map((folder) => folder.id))
  const byParent = new Map<string | null, Folder[]>()
  for (const folder of folders) {
    const parent = folder.parentId && ids.has(folder.parentId) ? folder.parentId : null
    byParent.set(parent, [...(byParent.get(parent) ?? []), folder])
  }
  const build = (parent: string | null): FolderNode[] =>
    (byParent.get(parent) ?? []).sort(compareFolders).map((folder) => {
      const children = build(folder.id)
      return {
        folder,
        children,
        unreadWithChildren: folder.unreadCount + children.reduce((total, child) => total + child.unreadWithChildren, 0),
      }
    })
  return build(null)
}

/** Remembers which accounts and folders the user collapsed; everything starts expanded. */
const STORAGE_KEY = 'mailx.sidebar.collapsed'

export function loadCollapsed(): Set<string> {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '[]')
    return new Set(Array.isArray(value) ? value.filter((item) => typeof item === 'string') : [])
  } catch {
    return new Set()
  }
}

export function saveCollapsed(collapsed: Set<string>) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify([...collapsed]))
}
