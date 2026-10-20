import type { ReactNode } from 'react'
import { Field, FieldContent, FieldDescription, FieldGroup, FieldLabel, FieldLegend, FieldSet, FieldTitle } from '@/components/ui/field'

/**
 * Shared building blocks so every settings page has the same rhythm:
 * a padded page, titled groups divided by a rule, and label/control rows.
 */

/** Scrollable body of one settings page; `footer` sticks to the bottom edge. */
export function SettingsPage({ children, footer }: { children: ReactNode; footer?: ReactNode }) {
  return (
    <div className="flex min-h-full flex-col">
      <div className="flex flex-col gap-6 px-6 pt-1 pb-6">{children}</div>
      {footer && (
        <footer className="sticky bottom-0 mt-auto flex items-center justify-between gap-4 border-t bg-background px-6 py-3">
          {footer}
        </footer>
      )}
    </div>
  )
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
