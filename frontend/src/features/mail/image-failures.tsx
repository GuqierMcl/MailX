import { useContext, useState } from 'react'
import { createPortal } from 'react-dom'
import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { Cancel01Icon, Image01Icon, RefreshIcon } from '@hugeicons/core-free-icons'
import { Button } from '@/components/ui/button'
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover'
import type { ImageFetchError, RemoteImageStatus } from '@/generated/accounts'
import { t } from '@/i18n'

import { ImageDetailsSlot, summarizeImageFailures } from './image-status'

const IMAGE_FAILURES: Record<ImageFetchError, string> = {
  get targetBlocked() { return t('mail:imageAddressRestricted') },
  get proxyPolicyRequired() { return t('mail:allowImageDomainResolution') },
  get dns() { return t('mail:imageDnsFailed') },
  get connect() { return t('mail:networkConnectionFailed') },
  get proxy() { return t('mail:proxyConnectionFailed') },
  get tls() { return t('mail:imageCertificateFailed') },
  get httpStatus() { return t('mail:imageServerNoImage') },
  get notFound() { return t('mail:imageServerNoImage') },
  get unsupportedType() { return t('mail:unsupportedImageFormat') },
  get invalidImage() { return t('mail:imageDecodeFailed') },
  get tooLarge() { return t('mail:imageTooLarge') },
  get timeout() { return t('mail:imageTimedOut') },
  get cancelled() { return t('mail:imageExpired') },
}

export function ImageFailures({ statuses, showing, settled, disabled, onRetry, onInspect }: {
  statuses: RemoteImageStatus[]; showing: boolean; settled: boolean; disabled: boolean; onRetry: () => void; onInspect: () => void
}) {
  useTranslation()
  const slot = useContext(ImageDetailsSlot)
  const [dismissed, setDismissed] = useState(false)
  const { failed, reasons, severe } = summarizeImageFailures(statuses)
  if (!showing || !failed) return null
  return <>
    {slot && createPortal(<Popover onOpenChange={(open) => { if (open) onInspect() }}>
      <PopoverTrigger render={<Button variant="ghost" size="icon-sm" className="text-muted-foreground"
        aria-label={t('mail:imageLoadingStatus')} title={t('mail:imageLoadingStatus')} />}>
        <HugeiconsIcon icon={Image01Icon} aria-hidden="true" />
      </PopoverTrigger>
      <PopoverContent align="end" aria-label={t('mail:imageLoadingStatus')} className="max-w-[calc(100vw-2rem)]">
        <p className="font-medium">{t('mail:imageFailureCount', { count: failed })}</p>
        <ul className="text-muted-foreground">{Array.from(reasons, (reason) => <li key={reason}>{IMAGE_FAILURES[reason]}</li>)}</ul>
        <Button variant="link" size="sm" className="self-start" disabled={disabled} onClick={onRetry}>
          <HugeiconsIcon icon={RefreshIcon} aria-hidden="true" />{t('mail:retryImages')}
        </Button>
      </PopoverContent>
    </Popover>, slot)}
    {settled && severe && !dismissed && <div className="flex shrink-0 flex-wrap items-center gap-x-2 text-xs text-muted-foreground">
      <span role="status">{t('mail:imagesUnavailable')}</span>
      <Button size="sm" variant="link" disabled={disabled} onClick={onRetry}>
        <HugeiconsIcon icon={RefreshIcon} aria-hidden="true" />{t('mail:retryImages')}
      </Button>
      <Button size="icon-sm" variant="ghost" aria-label={t('mail:dismissImageFailure')} onClick={() => setDismissed(true)}>
        <HugeiconsIcon icon={Cancel01Icon} aria-hidden="true" />
      </Button>
    </div>}
  </>
}
