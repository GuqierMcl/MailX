import { Fragment } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import { useTranslation } from 'react-i18next'
import type { EmailSummary } from '@/generated/accounts'
import { ContextMenuContent, ContextMenuItem, ContextMenuSeparator } from '@/components/ui/context-menu'
import { t } from '@/i18n'
import { emailMenuActions, type EmailMenuCallbacks } from './email-menu-actions'

type EmailContextMenuContentProps = EmailMenuCallbacks & {
  email: EmailSummary
  /** Where focus goes when the menu closes; see Base UI `finalFocus`. */
  finalFocus?: React.ComponentProps<typeof ContextMenuContent>['finalFocus']
}

/** The popup of the message list's right-click menu, for one message. */
export function EmailContextMenuContent({ email, finalFocus, ...callbacks }: EmailContextMenuContentProps) {
  useTranslation()
  const groups = emailMenuActions(email, callbacks)
  return (
    <ContextMenuContent aria-label={t('mail:messageMenu')} finalFocus={finalFocus} className="min-w-40">
      {groups.map((group, index) => (
        <Fragment key={group[0].kind}>
          {index > 0 && <ContextMenuSeparator />}
          {group.map((action) => (
            <ContextMenuItem
              key={action.kind}
              variant={action.destructive ? 'destructive' : 'default'}
              onClick={action.run}
            >
              <HugeiconsIcon
                icon={action.icon}
                fill={action.kind === 'unstar' ? 'currentColor' : 'none'}
              />
              {t(action.label)}
            </ContextMenuItem>
          ))}
        </Fragment>
      ))}
    </ContextMenuContent>
  )
}
