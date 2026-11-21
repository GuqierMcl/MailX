import { useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import { ErrorNotice } from '@/components/error-notice'
import { errorDetail } from '@/i18n/errors'
import { HugeiconsIcon } from '@hugeicons/react'
import { ArrowRight01Icon } from '@hugeicons/core-free-icons'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible'
import { Field, FieldContent, FieldDescription, FieldGroup, FieldLabel, FieldLegend, FieldSet, FieldTitle } from '@/components/ui/field'

/**
 * Shared building blocks so every settings page has the same rhythm:
 * a padded page, titled groups divided by a rule, and label/control rows.
 */

/** Scrollable body of one settings page; `footer` sticks to the bottom edge. */
export function SettingsPage({ children, footer, footerNotice }: { children: ReactNode; footer?: ReactNode; footerNotice?: ReactNode }) {
  return (
    <div className="flex min-h-full flex-col">
      <div className="flex flex-col gap-6 px-6 pt-1 pb-6">{children}</div>
      {footer && (
        <footer className="sticky bottom-0 mt-auto flex flex-wrap items-center justify-between gap-x-4 gap-y-2 border-t bg-background px-6 py-3">
          {footerNotice && <div className="w-full min-w-0">{footerNotice}</div>}
          {footer}
        </footer>
      )}
    </div>
  )
}

export function SettingsLoadError({ error, onRetry, message, actionLabel, disabled }: {
  error: unknown; onRetry: () => void; message?: string; actionLabel?: string; disabled?: boolean
}) {
  const { t } = useTranslation('settings')
  return <ErrorNotice layout="centered" message={message ?? t('readFailed')} detail={errorDetail(error)}
    actionLabel={actionLabel ?? t('retry')} onAction={onRetry} actionDisabled={disabled} />
}

/** Show a failed write beside the group that edited those settings. */
export function SettingsSaveError<T extends object>({ update, fields, message }: {
  update: { isError: boolean; error: unknown; variables?: T }
  fields?: readonly (keyof T)[]
  message?: string
}) {
  const { t } = useTranslation('settings')
  if (!update.isError || (fields && !fields.some((field) => Object.hasOwn(update.variables ?? {}, field)))) return null
  return <ErrorNotice message={message ?? t('saveFailedTitle')} detail={errorDetail(update.error)} />
}

/** A titled group of related settings; consecutive groups are separated by a rule. */
export function SettingsGroup({ title, badge, description, children }: {
  title: ReactNode
  badge?: ReactNode
  description?: ReactNode
  children: ReactNode
}) {
  return (
    // The rule lives on a wrapper: a fieldset border would run through its legend.
    <section className="border-t pt-6 first-of-type:border-t-0 first-of-type:pt-0">
      <FieldSet>
        <FieldLegend className="flex items-center gap-2">{title}{badge}</FieldLegend>
        {description && <FieldDescription className="-mt-2">{description}</FieldDescription>}
        <FieldGroup>{children}</FieldGroup>
      </FieldSet>
    </section>
  )
}

/**
 * A group that starts folded, for settings most people never need. The header
 * looks like a group title and is the toggle; `summary` stays visible beside it
 * while folded. The state is not remembered: the group is folded on every visit.
 * A caller that must open it on its own (to show a problem) controls it with
 * `open` and `onOpenChange`.
 */
export function SettingsDisclosure({ title, summary, open: controlled, onOpenChange, children }: {
  title: ReactNode
  summary?: ReactNode
  open?: boolean
  onOpenChange?: (open: boolean) => void
  children: ReactNode
}) {
  const [own, setOwn] = useState(false)
  const open = controlled ?? own
  return (
    <section className="border-t pt-6">
      <Collapsible open={open} onOpenChange={(next) => {
        setOwn(next)
        onOpenChange?.(next)
      }}>
        <CollapsibleTrigger className="group/disclosure flex w-full items-center gap-2 rounded-md text-left text-sm font-medium outline-none focus-visible:ring-2 focus-visible:ring-ring/30">
          {title}
          {!open && summary && <span className="min-w-0 truncate text-xs font-normal text-muted-foreground">{summary}</span>}
          <HugeiconsIcon icon={ArrowRight01Icon} strokeWidth={2} aria-hidden
            className="ml-auto size-4 shrink-0 text-muted-foreground transition-transform group-data-[panel-open]/disclosure:rotate-90" />
        </CollapsibleTrigger>
        <CollapsibleContent className="pt-6"><FieldGroup>{children}</FieldGroup></CollapsibleContent>
      </Collapsible>
    </section>
  )
}

/**
 * One setting with its label and help on the left and the control on the right;
 * stacks vertically when the page is narrow. Without `htmlFor`, the control
 * should name itself (e.g. a labelled radio group); without a control the row
 * is purely informational.
 */
export function SettingsRow({ label, description, htmlFor, children }: {
  label: ReactNode
  description?: ReactNode
  htmlFor?: string
  children?: ReactNode
}) {
  return (
    <Field orientation="responsive">
      <FieldContent>
        {htmlFor ? <FieldLabel htmlFor={htmlFor}>{label}</FieldLabel> : <FieldTitle>{label}</FieldTitle>}
        {description && <FieldDescription>{description}</FieldDescription>}
      </FieldContent>
      {children && <div className="flex shrink-0 items-center gap-2">{children}</div>}
    </Field>
  )
}
