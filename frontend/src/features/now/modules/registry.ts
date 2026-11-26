import { activityModule } from './activity'
import { attachmentsModule } from './attachments'
import { attentionModule } from './attention'
import { awayModule } from './away'
import { cleanupModule } from './cleanup'
import { codesModule } from './codes'
import { draftsModule } from './drafts'
import { inboxModule } from './inbox'
import type { NowModule, NowModuleId } from './types'

/**
 * Every module in its default order (now-page.md 4.3): the order when scores
 * tie, and the fixed order's tail for modules the user has not placed. A new
 * module goes here, in its zone's place, with its score rule and tests.
 */
export const NOW_MODULES: readonly NowModule[] = [
  attentionModule,
  codesModule,
  draftsModule,
  awayModule,
  inboxModule,
  cleanupModule,
  activityModule,
  attachmentsModule,
]

export const HIDEABLE_MODULES: readonly NowModule[] = NOW_MODULES.filter((module) => module.hideable)

const BY_ID = new Map<string, NowModule>(NOW_MODULES.map((module) => [module.id, module]))

export function moduleById(id: NowModuleId): NowModule {
  return BY_ID.get(id)!
}
