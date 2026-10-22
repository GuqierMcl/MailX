import { HugeiconsIcon } from '@hugeicons/react'
import { Tick02Icon } from '@hugeicons/core-free-icons'
import { Field, FieldDescription, FieldError, FieldLabel, FieldTitle } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { cn } from '@/lib/utils'
import { ACCOUNT_COLORS } from './account-color'
import { AccountAvatar } from './account-avatar'

type ProfileFieldsProps = {
  displayName: string
  color: string | null
  nameError?: string | null
  onDisplayNameChange(value: string): void
  /** Called when editing the name settles (blur or Enter). */
  onDisplayNameCommit?(): void
  onColorChange(color: string | null): void
  disabled?: boolean
}

/** Name and identification color: how the account looks in MailX, nothing more. */
export function ProfileFields({ displayName, color, nameError, onDisplayNameChange, onDisplayNameCommit, onColorChange, disabled }: ProfileFieldsProps) {
  const choices: { value: string | null; name: string }[] = [{ value: null, name: '无颜色' }, ...ACCOUNT_COLORS]
  return (
    <div className="flex flex-col gap-6">
      <Field data-invalid={!!nameError}>
        <FieldLabel htmlFor="displayName">账号名称</FieldLabel>
        <div className="flex items-center gap-3">
          <AccountAvatar displayName={displayName} color={color} className="size-8 text-sm" />
          <Input id="displayName" value={displayName} disabled={disabled} placeholder="例如：工作邮箱"
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
        {nameError ? <FieldError id="displayName-error">{nameError}</FieldError> : <FieldDescription>只用于 MailX 中的显示。</FieldDescription>}
      </Field>
      <Field>
        <FieldTitle id="account-color-label">标识颜色</FieldTitle>
        <div role="radiogroup" aria-labelledby="account-color-label" className="flex flex-wrap gap-2">
          {choices.map((choice) => {
            const checked = choice.value === color
            return (
              <button key={choice.value ?? 'none'} type="button" role="radio" aria-checked={checked} aria-label={choice.name} title={choice.name}
                disabled={disabled}
                onClick={() => !checked && onColorChange(choice.value)}
                style={choice.value ? { backgroundColor: choice.value } : undefined}
                className={cn(
                  'flex size-7 items-center justify-center rounded-full text-white outline-none transition-shadow focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 focus-visible:ring-offset-background disabled:opacity-50',
                  !choice.value && 'bg-muted text-muted-foreground ring-1 ring-border ring-inset',
                  checked && 'ring-2 ring-foreground ring-offset-2 ring-offset-background',
                )}>
                {checked && <HugeiconsIcon icon={Tick02Icon} className="size-4" strokeWidth={2.5} />}
              </button>
            )
          })}
        </div>
        <FieldDescription>在侧边栏和账号列表中区分不同邮箱。</FieldDescription>
      </Field>
    </div>
  )
}
