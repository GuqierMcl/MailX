import { t } from '@/i18n'
import { AccountAvatar } from '@/features/accounts/account-avatar'
import { canArchive } from '@/features/mail/use-archive-email'
import { MailRowCard } from '../mail-row-card'
import { StatButton, ViewAllUnread } from './inbox-view'
import type { NowModule } from './types'

/** Rows of unread inbox mail the page lists; more are a link away. */
export const UNREAD_ROWS = 6
/** This much new mail since the last visit puts the inbox ahead of "while you were away". */
export const BUSY_NEW_MAIL = 20

/** New, unread and starred mail, multi-account unread pills, and the newest unread messages with inline actions. */
export const inboxModule: NowModule = {
  id: 'inbox',
  zone: 'flow',
  hideable: true,
  fromOverview: true,
  title: () => t('now:inbox'),
  score: ({ data }) => (data.inbox.newMail >= BUSY_NEW_MAIL ? 295 : 260),
  signature: ({ inbox }) => `${inbox.newMail}|${inbox.unread}|${inbox.starredUnread}`,
  hasContent: ({ inbox }) => inbox.newMail > 0 || inbox.unread > 0 || inbox.starredUnread > 0 || inbox.rows.length > 0,
  render: ({ data, language, minute, mode, sinceTime, folders, mailActions, actions }) => {
    const { newMail, unread, starredUnread, rows, accountUnread = [] } = data.inbox
    const activeAccounts = accountUnread.filter((item) => item.unread > 0)
    return (
      <>
        <div className="flex flex-wrap items-center gap-2">
          {newMail > 0 && <StatButton value={newMail} label={t('now:statNew')} mode={mode} onClick={() => actions.onSelectMailbox({ kind: 'unified', role: 'inbox' })} />}
          <StatButton value={unread} label={t('now:statUnread')} mode={mode} onClick={actions.onOpenUnread} />
          {starredUnread > 0 && <StatButton value={starredUnread} label={t('now:starredUnread')} mode={mode} onClick={() => actions.onSelectMailbox({ kind: 'starred' })} />}
          {activeAccounts.length > 0 && (
            <div data-slot="now-account-pills" className="flex flex-wrap items-center gap-1.5 pl-1">
              {activeAccounts.map(({ account, folder, unread: count }) => (
                <button
                  key={account.id}
                  type="button"
                  onClick={() => actions.onSelectMailbox({ kind: 'folder', folder })}
                  aria-label={t('now:accountUnreadLabel', { account: account.displayName, count })}
                  className="flex max-w-44 items-center gap-1.5 rounded-full bg-muted/40 py-1 pr-2.5 pl-1.5 text-xs text-muted-foreground outline-none transition-colors hover:bg-foreground/8 hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
                >
                  <AccountAvatar
                    displayName={account.displayName}
                    color={account.color}
                    providerId={account.providerId}
                    useProviderIcon={account.useProviderIcon}
                    className="size-4 text-[9px]"
                  />
                  <span className="truncate">{account.displayName}</span>
                  <span className="shrink-0 font-semibold tabular-nums text-foreground">{count}</span>
                </button>
              ))}
            </div>
          )}
        </div>
        {rows.length > 0 && (
          <ul className="flex flex-col">
            {rows.map((email) => (
              <li key={email.id}>
                <MailRowCard
                  email={email}
                  language={language}
                  now={minute}
                  isNew={Date.parse(email.receivedAt) > sinceTime}
                  onOpen={actions.onOpenEmail}
                  quickActions={mailActions && !email.localOnly ? {
                    busy: mailActions.busy.has(email.id),
                    onMarkRead: () => void mailActions.markRead(email),
                    onToggleStar: () => void mailActions.toggleStar(email),
                    onArchive: canArchive(email, folders) ? () => void mailActions.archive(email) : undefined,
                  } : undefined}
                />
              </li>
            ))}
          </ul>
        )}
        {unread > UNREAD_ROWS && <ViewAllUnread unread={unread} onOpenUnread={actions.onOpenUnread} />}
      </>
    )
  },
}
