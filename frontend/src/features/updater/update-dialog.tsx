import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { t } from '@/i18n'
import { Button } from '@/components/ui/button'
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog'
import type { MailxClient } from '@/lib/mailx-client'
import { releaseDate, releaseUrl, restartBlockedText, retry as retryUpdate } from './details'
import { UpdateNotes } from './update-notes'
import { downloadPercent, updateVersion, type UpdateSnapshot } from './types'

/**
 * The update modal. It shows the phase the host reports; with `canInstall`
 * false (a notify-only edition) it only explains how to get the release.
 */
export function UpdateDialog({ client, snapshot, open, onOpenChange }: {
  client: MailxClient
  snapshot: UpdateSnapshot
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  useTranslation()
  const version = updateVersion(snapshot)
  if (!version) return null
  const percent = downloadPercent(snapshot)
  const info = 'notes' in snapshot ? snapshot : null
  const date = info ? releaseDate(info.date) : null
  const openLink = (url: string) => void client.openUpdateLink(url).catch(() => undefined)
  const failed = snapshot.phase === 'failed' ? snapshot : null
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="flex max-h-[min(560px,calc(100dvh-4rem))] flex-col gap-0 p-0 sm:max-w-lg">
        <DialogHeader className="border-b px-5 py-4">
          <DialogTitle>{t('updater:dialogTitle', { version })}</DialogTitle>
          <DialogDescription>
            {info ? t('updater:fromTo', { current: info.current, version }) : null}
            {date && <span className="ml-2">{t('updater:published', { date })}</span>}
          </DialogDescription>
        </DialogHeader>
        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
          {info?.notes
            ? <UpdateNotes notes={info.notes} onOpenLink={openLink} truncatedHint={t('updater:notesTruncated')} />
            : <p className="text-sm text-muted-foreground">{t('updater:noNotes')}</p>}
          {!snapshot.canInstall && <p className="mt-3 text-xs text-muted-foreground">{t('updater:installUnavailable')}</p>}
        </div>
        <DialogFooter className="flex-col items-stretch gap-2 border-t px-5 py-3 sm:flex-row sm:items-center">
          {failed && <p className="mr-auto text-xs text-destructive">{t(failed.stage === 'check' ? 'updater:checkFailed' : 'updater:downloadFailed')}</p>}
          {snapshot.phase === 'downloading' && (
            <div className="mr-auto flex w-full flex-col gap-1 sm:max-w-52">
              <p className="text-xs text-muted-foreground" aria-live="polite">
                {percent === null ? t('updater:downloading') : t('updater:downloadingPercent', { percent })}
              </p>
              <div role="progressbar" aria-label={t('updater:downloading')} aria-valuemin={0} aria-valuemax={100}
                aria-valuenow={percent ?? undefined}
                className="h-1 overflow-hidden rounded-full bg-muted">
                <div className="h-full rounded-full bg-primary transition-[width] duration-150" style={{ width: `${percent ?? 0}%` }} />
              </div>
            </div>
          )}
          <Button variant="ghost" size="sm" className="sm:mr-auto" onClick={() => openLink(releaseUrl(version))}>
            {t('updater:viewOnGitHub')}
          </Button>
          <Actions client={client} snapshot={snapshot} onClose={() => onOpenChange(false)} />
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function Actions({ client, snapshot, onClose }: { client: MailxClient; snapshot: UpdateSnapshot; onClose: () => void }) {
  useTranslation()
  const [restartRefusal, setRestartRefusal] = useState<string | null>(null)
  const run = (operation: Promise<unknown>) => void operation.catch(() => undefined)
  switch (snapshot.phase) {
    case 'available':
      return snapshot.canInstall ? (
        <>
          <Button variant="outline" size="sm" onClick={onClose}>{t('updater:later')}</Button>
          <Button size="sm" onClick={() => run(client.downloadUpdate())}>{t('updater:download')}</Button>
        </>
      ) : (
        <Button size="sm" onClick={onClose}>{t('updater:close')}</Button>
      )
    case 'downloading':
      return (
        <>
          <Button variant="outline" size="sm" onClick={onClose}>{t('updater:continueInBackground')}</Button>
          <Button variant="outline" size="sm" onClick={() => run(client.cancelUpdate())}>{t('updater:cancelDownload')}</Button>
        </>
      )
    case 'ready':
      // Restarting now ends the app, so a send in flight has to finish first:
      // the host refuses and says so here, and the update waits for the next quit.
      return (
        <>
          {restartRefusal && <p className="mr-auto text-xs text-destructive">{restartRefusal}</p>}
          <Button variant="outline" size="sm" onClick={onClose}>{t('updater:installOnQuit')}</Button>
          <Button size="sm" onClick={() => {
            client.restartToUpdate().catch((error: unknown) => setRestartRefusal(restartBlockedText(error) ?? t('updater:restartFailed')))
          }}>{t('updater:restartNow')}</Button>
        </>
      )
    case 'failed':
      return (
        <>
          <Button variant="outline" size="sm" onClick={onClose}>{t('updater:later')}</Button>
          <Button size="sm" onClick={() => run(retryUpdate(client, snapshot.stage))}>{t('updater:retry')}</Button>
        </>
      )
    default:
      return <Button variant="outline" size="sm" onClick={onClose}>{t('updater:close')}</Button>
  }
}

