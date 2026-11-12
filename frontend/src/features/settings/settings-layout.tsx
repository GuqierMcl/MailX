import { useState, type ReactNode } from 'react'
import { HugeiconsIcon } from '@hugeicons/react'
import { ArrowRight01Icon } from '@hugeicons/core-free-icons'
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '@/components/ui/collapsible'
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
 * A group that starts folded, for settings most people never need. The header
 * looks like a group title and is the toggle; `summary` stays visible beside it
 * while folded. The state is not remembered: the group is folded on every visit.
 */
export function SettingsDisclosure({ title, summary, children }: {
  title: ReactNode
  summary?: ReactNode
  children: ReactNode
}) {
  const [open, setOpen] = useState(false)
  return (
    <section className="border-t pt-6">
      <Collapsible open={open} onOpenChange={setOpen}>
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
