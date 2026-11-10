import type { UpdateSnapshot } from './types'

/** A version worth telling the user about, once the previous state differs. */
export type Announcement =
  | { kind: 'available'; version: string; manual: boolean }
  | { kind: 'ready'; version: string; manual: boolean }
  | { kind: 'installFailed'; version: string; manual: boolean }

/**
 * What changed between two states, from the user's point of view. The caller
 * decides what to do with it; keeping the decision here makes "once per
 * version, once per run" testable without a host.
 */
export function announcements(previous: UpdateSnapshot | null, next: UpdateSnapshot, manual: boolean): Announcement[] {
  const found = (state: UpdateSnapshot | null) => (state?.phase === 'available' ? state.version : null)
  const ready = (state: UpdateSnapshot | null) => (state?.phase === 'ready' ? state.version : null)
  const installFailed = (state: UpdateSnapshot | null) => (state?.phase === 'failed' && state.stage === 'install' ? state.version : null)
  const out: Announcement[] = []
  if (found(next) && found(next) !== found(previous)) out.push({ kind: 'available', version: found(next)!, manual })
  if (ready(next) && ready(next) !== ready(previous)) out.push({ kind: 'ready', version: ready(next)!, manual })
  if (installFailed(next) && installFailed(next) !== installFailed(previous)) out.push({ kind: 'installFailed', version: installFailed(next)!, manual })
  return out
}

/**
 * Whether an announcement still needs to be delivered: a version is announced
 * at most once per stage and run, only while the modal is shut.
 */
export function shouldAnnounce(notice: Announcement, announced: ReadonlySet<string>, modalOpen: boolean): boolean {
  // A manual check is not a notification: the user is already looking.
  return !notice.manual && !modalOpen && !announced.has(announcementKey(notice))
}

export function announcementKey(notice: Announcement): string {
  return `${notice.kind}:${notice.version}`
}
