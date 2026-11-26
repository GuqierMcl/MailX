import { useState } from 'react'
import type { NowSettings } from '@/generated/accounts'
import { orderModules } from './arrange'
import { NOW_MODULES } from './registry'
import type { NowContext, NowModuleId } from './types'

/**
 * The order of this stay on the page (now-page.md 4.3). Worked out once, when
 * the page is ready; later data changes do not move anything, and a module
 * that gains content appears at its slot. Only a change of the arrangement
 * setting itself (made in the settings dialog over the page) orders again.
 * Null until `ready`.
 */
export function useModuleOrder(ready: boolean, settings: Pick<NowSettings, 'arrangement' | 'order'>, context: NowContext): NowModuleId[] | null {
  const key = JSON.stringify([settings.arrangement, settings.order])
  const [saved, setSaved] = useState<{ key: string; order: NowModuleId[] } | null>(null)
  if (ready && saved?.key !== key) setSaved({ key, order: orderModules(NOW_MODULES, context, settings) })
  return saved?.order ?? null
}
