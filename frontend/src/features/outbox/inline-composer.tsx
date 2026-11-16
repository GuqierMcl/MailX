import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { Cancel01Icon, SquareArrowUpRightIcon } from '@hugeicons/core-free-icons'
import type { Account } from '@/generated/accounts'
import { IconButton } from '@/components/icon-button'
import { Composer } from '@/features/composer/composer'
import type { MailxClient } from '@/lib/mailx-client'
import type { InlineComposer } from './use-inline-composer'

/**
 * The composer inside the main window's reader (sending.md section 17.7): the
 * same `Composer` as in a window, with "在新窗口中打开" and "关闭" in its header.
 * It fades in over whatever the reader showed. Esc does nothing here on purpose.
 */
export function InlineComposerPane({ client, accounts, inline }: {
  client: MailxClient
  /** In display order (`layout.accountOrder`), as the sender picker lists them. */
  accounts: Account[]
  inline: InlineComposer
}) {
  useTranslation()
  const draft = inline.current
  if (!draft) return null
  return (
    <div
      key={draft.key}
      data-slot="inline-composer"
      className="flex h-full min-h-0 animate-in flex-col duration-200 ease-out fade-in-0 motion-reduce:animate-none"
      inert={inline.busy}
    >
      <Composer
        client={client}
        draftId={draft.draftId}
        accounts={accounts}
        controllerRef={inline.controllerRef}
        onSent={inline.close}
        onDiscarded={inline.close}
        headerActions={(
          <>
            <IconButton label={t('mail:inlineComposePopOut')} onClick={() => void inline.popOut()}>
              <HugeiconsIcon icon={SquareArrowUpRightIcon} />
            </IconButton>
            <IconButton label={t('mail:inlineComposeClose')} onClick={() => void inline.leave()}>
              <HugeiconsIcon icon={Cancel01Icon} />
            </IconButton>
          </>
        )}
      />
    </div>
  )
}
