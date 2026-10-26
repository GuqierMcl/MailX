import { t } from '@/i18n'
import { clientFailureCode, type MailxClient } from '@/lib/mailx-client'
import { downloadPercent, type UpdateSnapshot } from './types'

const dateFormat = new Intl.DateTimeFormat(undefined, { dateStyle: 'medium' })

/** Where the release page lives; the "view on GitHub" link under the notes. */
export function releaseUrl(version: string): string {
  return `https://github.com/GuqierMcl/MailX/releases/tag/v${encodeURIComponent(version)}`
}

export function releaseDate(date: number | null): string | null {
  if (date === null) return null
  return dateFormat.format(new Date(date * 1000))
}

/** The label doubles as the aria-label and the tooltip; it always names the version. */
export function indicatorLabel(snapshot: UpdateSnapshot): string | null {
  switch (snapshot.phase) {
    case 'available':
      return t('updater:indicatorAvailable', { version: snapshot.version })
    case 'downloading':
      return t('updater:indicatorDownloading', { version: snapshot.version, percent: downloadPercent(snapshot) ?? 0 })
    case 'ready':
      return t('updater:indicatorReady', { version: snapshot.version })
    default:
      return null
  }
}

/** Retries the step that failed; a failed download keeps its version. */
export function retry(client: MailxClient, stage: 'check' | 'download') {
  return stage === 'check' ? client.checkForUpdates() : client.downloadUpdate()
}

/** The refusal shown next to "restart now" when mail is still being sent. */
export function restartBlockedText(error: unknown): string | null {
  return clientFailureCode(error) === 'alreadySending' ? t('updater:sendingBlocksRestart') : null
}
