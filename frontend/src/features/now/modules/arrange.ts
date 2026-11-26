import type { NowSettings } from '@/generated/accounts'
import type { Snoozed } from './snooze'
import type { NowContext, NowData, NowModule, NowModuleId } from './types'

/** The module that stays first and ignores the fixed order. */
export const PINNED_ID: NowModuleId = 'attention'

/**
 * The fixed order of the ids a registry knows: the user's order first (ids
 * the registry lacks are ignored, repeats count once), the rest in registry
 * order after it. The pinned module is not part of it.
 */
export function fixedOrder(ids: readonly string[], order: readonly string[]): string[] {
  const known = new Set(ids.filter((id) => id !== PINNED_ID))
  const result: string[] = []
  for (const id of order) {
    if (known.has(id) && !result.includes(id)) result.push(id)
  }
  for (const id of known) {
    if (!result.includes(id)) result.push(id)
  }
  return result
}

/**
 * Every module in the order this visit shows them, whether or not it has
 * content now (the page fills slots as content arrives). Automatic: by score,
 * highest first, then registry order; the pinned module needs no score and
 * leads (a zone's base only keeps scores close to it, see now-page.md 4.3).
 * Fixed: the user's order with the pinned module first.
 */
export function orderModules(modules: readonly NowModule[], context: NowContext, settings: Pick<NowSettings, 'arrangement' | 'order'>): NowModuleId[] {
  if (settings.arrangement === 'fixed') {
    const ids = modules.map((module) => module.id)
    const pinned = ids.filter((id) => id === PINNED_ID)
    return [...pinned, ...fixedOrder(ids, settings.order)] as NowModuleId[]
  }
  return modules
    .map((module, position) => ({ module, position, score: module.id === PINNED_ID ? Infinity : module.score(context) }))
    .sort((a, b) => (a.score === b.score ? a.position - b.position : b.score - a.score))
    .map(({ module }) => module.id)
}

/**
 * Whether the user keeps a module out of sight right now: hidden for good,
 * or hidden for today with the content unchanged since. A module that is not
 * hideable never is.
 */
export function suppressed(module: NowModule, settings: Pick<NowSettings, 'hidden'>, snoozed: Snoozed, data: NowData, today: string): boolean {
  if (!module.hideable) return false
  if (settings.hidden.includes(module.id)) return true
  const entry = snoozed[module.id]
  return entry !== undefined && entry.day === today && entry.signature === module.signature(data)
}

/** This visit's order without what the user hides or snoozed. */
export function arrange(modules: readonly NowModule[], context: NowContext, settings: Pick<NowSettings, 'arrangement' | 'order' | 'hidden'>, snoozed: Snoozed, today: string): NowModuleId[] {
  const byId = new Map(modules.map((module) => [module.id, module]))
  return orderModules(modules, context, settings).filter((id) => !suppressed(byId.get(id)!, settings, snoozed, context.data, today))
}
