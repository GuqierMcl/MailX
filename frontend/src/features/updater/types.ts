/**
 * What the update UI may show, as the host reports it.
 *
 * `canInstall` is a capability of the edition, not a phase: the desktop host
 * downloads, verifies and installs; a server or web edition would only announce
 * the version and the notes. Every piece of update UI branches on it and never
 * assumes a download can follow.
 */
export type UpdatePhase = 'idle' | 'checking' | 'available' | 'downloading' | 'ready' | 'failed'

/** Which step failed, so "retry" can do the right thing. */
export type UpdateFailureStage = 'check' | 'download'

/** Release metadata stays available throughout downloading, completion and retry. */
export type ReleaseDetails = {
  current: string
  notes: string | null
  /** Publication time in seconds since the Unix epoch, when the manifest had one. */
  date: number | null
}

export type UpdateSnapshot =
  | { canInstall: boolean; phase: 'idle' }
  | { canInstall: boolean; phase: 'checking' }
  | ({ canInstall: boolean; phase: 'available'; version: string } & ReleaseDetails)
  | ({ canInstall: boolean; phase: 'downloading'; version: string; downloaded: number; total: number | null } & ReleaseDetails)
  | ({ canInstall: boolean; phase: 'ready'; version: string } & ReleaseDetails)
  | {
    canInstall: boolean
    phase: 'failed'
    stage: 'check'
    version: null
  }
  | ({ canInstall: boolean; phase: 'failed'; stage: 'download'; version: string } & ReleaseDetails)

export const IDLE_UPDATE: UpdateSnapshot = { canInstall: false, phase: 'idle' }

/** The version a snapshot talks about, if any. */
export function updateVersion(snapshot: UpdateSnapshot): string | null {
  switch (snapshot.phase) {
    case 'available':
    case 'downloading':
    case 'ready':
      return snapshot.version
    case 'failed':
      return snapshot.version
    default:
      return null
  }
}

/** Whole percent of the download, when the host knows the total size. */
export function downloadPercent(snapshot: UpdateSnapshot): number | null {
  if (snapshot.phase !== 'downloading' || !snapshot.total || snapshot.total <= 0) return null
  return Math.min(100, Math.floor((snapshot.downloaded * 100) / snapshot.total))
}
