import { HugeiconsIcon } from '@hugeicons/react'
import { Copy01Icon, Tick02Icon } from '@hugeicons/core-free-icons'
import { t } from '@/i18n'
import { IconButton } from '@/components/icon-button'
import { displayName } from '@/features/mail/format'
import type { VerificationCodes } from '@/features/mail/use-verification-codes'
import { relativeTime } from '../relative-time'
import type { NowRenderEnv, ShownCode } from './types'

/**
 * One code the shared recognizer confirmed. Copying goes through the shared
 * flow: the host writes the clipboard, the move-to-Trash setting applies and
 * the notices match the list and reader.
 */
export function CodeRow({ code, language, now, verification, onOpenEmail }: { code: ShownCode; language: string; now: number; verification: VerificationCodes; onOpenEmail: NowRenderEnv['actions']['onOpenEmail'] }) {
  const { ready } = code
  const shown = ready.code
  const busy = verification.busy(code.emailId)
  const copied = verification.copied(code.emailId, ready.recognitionId)
  const label = busy ? t('mail:verificationCopying') : copied ? t('mail:verificationCopiedShort') : t('now:copyCode', { code: shown })
  return (
    <li className="flex items-center gap-3 rounded-lg px-3 py-2">
      <p className="font-mono text-2xl font-semibold tracking-[0.18em] tabular-nums">
        <span className="sr-only">{t('now:codeLabel', { code: shown })}</span>
        <span aria-hidden>{shown}</span>
      </p>
      <div className="min-w-0 flex-1 text-sm">
        <button
          type="button"
          className="max-w-full truncate rounded-sm text-left font-medium outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring"
          onClick={() => onOpenEmail({ id: code.emailId, accountId: code.accountId })}
        >
          {displayName(code.from)}
        </button>
        <p className="text-xs text-muted-foreground">{relativeTime(code.receivedAt, language, now)}</p>
      </div>
      <IconButton
        label={label}
        size="icon-sm"
        disabled={busy}
        onClick={() => void verification.copy(code.emailId, ready.recognitionId)}
      >
        <HugeiconsIcon icon={copied ? Tick02Icon : Copy01Icon} />
      </IconButton>
    </li>
  )
}
