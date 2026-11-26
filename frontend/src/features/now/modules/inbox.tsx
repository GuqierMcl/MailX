import { t } from '@/i18n'
import { MailRowCard } from '../mail-row-card'
import { StatButton, ViewAllUnread } from './inbox-view'
import type { NowModule } from './types'

/** Rows of unread inbox mail the page lists; more are a link away. */
export const UNREAD_ROWS = 6
/** This much new mail since the last visit puts the inbox ahead of "while you were away". */
export const BUSY_NEW_MAIL = 20

/** New, unread and starred mail, and the newest unread messages. */
export const inboxModule: NowModule = {
  id: 'inbox',
  zone: 'flow',
  hideable: true,
  fromOverview: true,
  title: () => t('now:inbox'),
  score: ({ data }) => (data.inbox.newMail >= BUSY_NEW_MAIL ? 295 : 260),
  signature: ({ inbox }) => `${inbox.newMail}|${inbox.unread}|${inbox.starredUnread}`,
  hasContent: ({ inbox }) => inbox.newMail > 0 || inbox.unread > 0 || inbox.starredUnread > 0 || inbox.rows.length > 0,
  render: ({ data, language, minute, mode, sinceTime, actions }) => {
    const { newMail, unread, starredUnread, rows } = data.inbox
    return (
      <>
        <div className="flex flex-wrap gap-2">
          {newMail > 0 && <StatButton value={newMail} label={t('now:statNew')} mode={mode} onClick={() => actions.onSelectMailbox({ kind: 'unified', role: 'inbox' })} />}
          <StatButton value={unread} label={t('now:statUnread')} mode={mode} onClick={actions.onOpenUnread} />
          {starredUnread > 0 && <StatButton value={starredUnread} label={t('now:starredUnread')} mode={mode} onClick={() => actions.onSelectMailbox({ kind: 'starred' })} />}
        </div>
        {rows.length > 0 && (
          <ul className="flex flex-col">
            {rows.map((email) => (
              <li key={email.id}>
                <MailRowCard email={email} language={language} now={minute} isNew={Date.parse(email.receivedAt) > sinceTime} onOpen={actions.onOpenEmail} />
              </li>
            ))}
          </ul>
        )}
        {unread > UNREAD_ROWS && <ViewAllUnread unread={unread} onOpenUnread={actions.onOpenUnread} />}
      </>
    )
  },
}
