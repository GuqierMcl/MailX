import { t } from '@/i18n'
import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { Cancel01Icon, Tick02Icon } from '@hugeicons/core-free-icons'
import { Field, FieldContent, FieldDescription, FieldError, FieldLabel, FieldTitle } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Switch } from '@/components/ui/switch'
import { cn } from '@/lib/utils'
import { ACCOUNT_COLORS } from './account-color'
import { AccountAvatar } from './account-avatar'
import { providerOf } from './providers'
import type { ClientFailure } from '@/lib/mailx-client'
import { errorText } from '@/i18n/errors'

type ProfileFieldsProps = {
  displayName: string
  color: string | null
  providerId?: string | null
  useProviderIcon?: boolean
  onUseProviderIconChange?(value: boolean): void
  nameError?: ClientFailure | null
  /** The name recipients see in `From`; empty sends with the bare address. */
  senderName: string
  senderNameError?: ClientFailure | null
  onSenderNameChange(value: string): void
  /** Called when editing the sender name settles (blur or Enter). */
  onSenderNameCommit?(): void
  onDisplayNameChange(value: string): void
  /** Called when editing the name settles (blur or Enter). */
  onDisplayNameCommit?(): void
  onColorChange(color: string | null): void
  disabled?: boolean
}

/** Name and identification color: how the account looks in MailX, nothing more. */
export function ProfileFields({ displayName, color, providerId, useProviderIcon = true, onUseProviderIconChange, nameError, senderName, senderNameError, onSenderNameChange, onSenderNameCommit, onDisplayNameChange, onDisplayNameCommit, onColorChange, disabled }: ProfileFieldsProps) {
  useTranslation()
  const choices: { value: string | null; name: string }[] = [{ value: null, name: t('accounts:noColor') }, ...ACCOUNT_COLORS]
  return (
    <div className="flex flex-col gap-6">
      <Field data-invalid={!!nameError}>
        <FieldLabel htmlFor="displayName">{t('accounts:accountName')}</FieldLabel>
        <div className="flex items-center gap-3">
          <AccountAvatar displayName={displayName} color={color} providerId={providerId} useProviderIcon={useProviderIcon} className="size-8 text-sm" />
          <Input id="displayName" value={displayName} disabled={disabled} placeholder={t('accounts:accountNamePlaceholder')}
            aria-invalid={!!nameError} aria-describedby={nameError ? 'displayName-error' : undefined}
            onChange={(event) => onDisplayNameChange(event.target.value)}
            onBlur={() => onDisplayNameCommit?.()}
            onKeyDown={(event) => {
              if (event.key === 'Enter') {
                event.preventDefault()
                onDisplayNameCommit?.()
              }
            }} />
        </div>
        {nameError ? <FieldError id="displayName-error">{errorText(nameError)}</FieldError> : <FieldDescription>{t('accounts:accountNameHint')}</FieldDescription>}
      </Field>
      <Field data-invalid={!!senderNameError}>
        <FieldLabel htmlFor="senderName">{t('accounts:senderName')}</FieldLabel>
        <Input id="senderName" value={senderName} disabled={disabled} placeholder={t('accounts:senderNamePlaceholder')} maxLength={128}
          autoComplete="off"
          aria-invalid={!!senderNameError} aria-describedby={senderNameError ? 'senderName-error' : undefined}
          onChange={(event) => onSenderNameChange(event.target.value)}
          onBlur={() => onSenderNameCommit?.()}
          onKeyDown={(event) => {
            if (event.key === 'Enter') {
              event.preventDefault()
              onSenderNameCommit?.()
            }
          }} />
        {senderNameError ? <FieldError id="senderName-error">{errorText(senderNameError)}</FieldError> : <FieldDescription>{t('accounts:senderNameHint')}</FieldDescription>}
      </Field>
      {providerId && providerOf(providerId).logo && onUseProviderIconChange && (
        <Field orientation="horizontal">
          <FieldContent>
            <FieldLabel htmlFor="useProviderIcon">{t('accounts:useProviderIcon')}</FieldLabel>
            <FieldDescription>{t('accounts:useProviderIconHint')}</FieldDescription>
          </FieldContent>
          <Switch id="useProviderIcon" checked={useProviderIcon} onCheckedChange={onUseProviderIconChange} disabled={disabled} />
        </Field>
      )}
      <Field>
        <FieldTitle id="account-color-label">{t('accounts:identificationColor')}</FieldTitle>
        <div role="radiogroup" aria-labelledby="account-color-label" className="flex flex-wrap gap-2">
          {choices.map((choice) => {
            const checked = choice.value === color
            // A pick confirms even the current default as the user's own choice.
            return (
              <button key={choice.value ?? 'none'} type="button" role="radio" aria-checked={checked} aria-label={choice.name} title={choice.name}
                disabled={disabled}
                onClick={() => onColorChange(choice.value)}
                style={choice.value ? { backgroundColor: choice.value } : undefined}
                className={cn(
                  'flex size-7 items-center justify-center rounded-full outline-none transition-shadow focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:opacity-50',
                  choice.value ? 'text-white' : 'border border-border text-muted-foreground',
                  checked && 'ring-2 ring-foreground ring-offset-2 ring-offset-background',
                )}>
                {choice.value
                  ? checked && <HugeiconsIcon icon={Tick02Icon} aria-hidden className="size-4" strokeWidth={2.5} />
                  : <HugeiconsIcon icon={Cancel01Icon} aria-hidden className="size-4" strokeWidth={2} />}
              </button>
            )
          })}
        </div>
        <FieldDescription>{t('accounts:identificationColorHint')}</FieldDescription>
      </Field>
    </div>
  )
}
