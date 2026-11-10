import { useId, useState, type MouseEvent } from 'react'
import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { ArrowUpRight01Icon, Bug01Icon, BulbIcon, GithubIcon, News01Icon } from '@hugeicons/core-free-icons'
import { Button } from '@/components/ui/button'
import { FieldGroup } from '@/components/ui/field'
import { AutoUpdateRow, UpdateStatusRow } from '@/features/updater/about-updates'
import type { MailxClient } from '@/lib/mailx-client'
import { SettingsPage } from './settings-layout'

const PROJECT_LINKS = [
  { key: 'github', url: 'https://github.com/GuqierMcl/MailX', icon: GithubIcon },
  { key: 'bug', url: 'https://github.com/GuqierMcl/MailX/issues/new?template=bug_report.md', icon: Bug01Icon },
  { key: 'feature', url: 'https://github.com/GuqierMcl/MailX/issues/new?template=feature_request.md', icon: BulbIcon },
  { key: 'releases', url: 'https://github.com/GuqierMcl/MailX/releases', icon: News01Icon },
] as const

export function AboutPane({ client }: { client: MailxClient }) {
  const { t } = useTranslation('settings')
  const id = useId()

  return (
    <SettingsPage>
      <div className="flex items-center gap-4">
        <img src={`${import.meta.env.BASE_URL}logo.svg`} alt={t('aboutPage.logo')} width={64} height={64} className="size-16 shrink-0" />
        <div className="min-w-0">
          <h3 className="text-2xl font-semibold tracking-tight">MailX</h3>
          <p className="mt-1 text-xs text-muted-foreground">{t('version')} <span className="select-text tabular-nums">{__APP_VERSION__}</span></p>
        </div>
      </div>
      <section aria-labelledby={`${id}-updates`} className="flex flex-col gap-4">
        <h3 id={`${id}-updates`} className="text-xs font-medium text-muted-foreground">{t('aboutPage.updates')}</h3>
        <FieldGroup>
          <UpdateStatusRow />
          <AutoUpdateRow client={client} />
        </FieldGroup>
      </section>
      <section aria-labelledby={`${id}-project`} className="flex flex-col gap-2">
        <h3 id={`${id}-project`} className="text-xs font-medium text-muted-foreground">{t('aboutPage.project')}</h3>
        <ul className="-mx-2">
          {PROJECT_LINKS.map((link) => <ProjectLink key={link.key} client={client} link={link} externalId={`${id}-external`} />)}
        </ul>
        <span id={`${id}-external`} className="sr-only">{t('aboutPage.external')}</span>
      </section>
    </SettingsPage>
  )
}

function ProjectLink({ client, link, externalId }: {
  client: MailxClient
  link: (typeof PROJECT_LINKS)[number]
  externalId: string
}) {
  const { t } = useTranslation('settings')
  const id = useId()
  const [failed, setFailed] = useState(false)
  const [copyState, setCopyState] = useState<'idle' | 'copying' | 'copied' | 'failed'>('idle')

  const openLink = (event: MouseEvent<HTMLAnchorElement>) => {
    // Keep primary, keyboard and middle-click navigation in the system browser.
    event.preventDefault()
    setFailed(false)
    setCopyState('idle')
    void client.openExternalLink(link.url).catch(() => setFailed(true))
  }

  const copyLink = async () => {
    setCopyState('copying')
    try {
      await navigator.clipboard.writeText(link.url)
      setCopyState('copied')
    } catch {
      setCopyState('failed')
    }
  }

  return (
    <li>
      <a href={link.url} target="_blank" rel="noopener noreferrer"
        onClick={openLink}
        onAuxClick={(event) => { if (event.button === 1) openLink(event) }}
        aria-labelledby={`${id}-title`} aria-describedby={`${id}-description ${externalId}`}
        className="flex min-h-14 items-center gap-3 rounded-lg px-2 py-2.5 transition-colors hover:bg-muted/60 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring">
        <HugeiconsIcon icon={link.icon} size={18} className="shrink-0 text-muted-foreground" aria-hidden="true" />
        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
          <span id={`${id}-title`} className="text-sm font-medium">{t(`aboutPage.${link.key}.title`)}</span>
          <span id={`${id}-description`} className="text-xs text-muted-foreground">{t(`aboutPage.${link.key}.description`)}</span>
        </span>
        <HugeiconsIcon icon={ArrowUpRight01Icon} size={16} className="shrink-0 text-muted-foreground/70" aria-hidden="true" />
      </a>
      {failed && <div className="ml-9 flex flex-col items-start gap-2 px-2 pb-3 text-xs">
        <div role="alert">
          <p className="text-destructive">{t('aboutPage.openFailed')}</p>
          <p className="mt-0.5 text-muted-foreground">{t('aboutPage.openFailedHint')}</p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button variant="outline" size="sm" disabled={copyState === 'copying'} onClick={() => void copyLink()}>{t('aboutPage.copyLink')}</Button>
          <span role="status" className={copyState === 'failed' ? 'text-destructive' : 'text-muted-foreground'}>
            {copyState === 'copied' ? t('aboutPage.linkCopied') : copyState === 'failed' ? t('aboutPage.copyFailed') : null}
          </span>
        </div>
        {copyState === 'failed' && <code className="w-full break-all select-text">{link.url}</code>}
      </div>}
    </li>
  )
}
