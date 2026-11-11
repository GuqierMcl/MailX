import { useCallback, useMemo } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import type { EmailSummary } from '@/generated/accounts'
import { toast } from '@/components/ui/toast'
import { errorText } from '@/i18n/errors'
import { t } from '@/i18n'
import type { MailxClient } from '@/lib/mailx-client'
import { contactKeys, mailKeys } from '@/lib/query-client'
import { displayName } from './format'

/**
 * Which senders have their mail grouped, and the switch for it. A change refreshes the
 * rules and every cached list (rows of that sender merge into one, or split again), and
 * is announced with an undo.
 */
export function useContactAggregation(client: MailxClient) {
  const queryClient = useQueryClient()
  const rules = useQuery({
    queryKey: contactKeys.aggregations(),
    queryFn: () => client.listContactAggregations(),
  })
  const grouped = useMemo(
    () => new Set((rules.data ?? []).map((rule) => rule.address.trim().toLowerCase())),
    [rules.data],
  )
  const isAggregated = useCallback(
    (email: EmailSummary) => grouped.has(email.from.address.trim().toLowerCase()),
    [grouped],
  )
  const setAggregated = useCallback((email: EmailSummary, aggregate: boolean) => {
    const address = email.from.address
    const name = displayName(email.from)
    const apply = async (wanted: boolean, canUndo: boolean) => {
      try {
        await client.setContactAggregation(address, wanted)
      } catch (error) {
        toast.error(t(wanted ? 'mail:groupContactFailed' : 'mail:ungroupContactFailed'), { description: errorText(error) })
        return
      }
      void queryClient.invalidateQueries({ queryKey: contactKeys.aggregations() })
      void queryClient.invalidateQueries({ queryKey: mailKeys.emailLists() })
      toast.success(t(wanted ? 'mail:contactGrouped' : 'mail:contactUngrouped', { name }), canUndo
        ? { action: { label: t('mail:undo'), onClick: () => void apply(!wanted, false) } }
        : undefined)
    }
    void apply(aggregate, true)
  }, [client, queryClient])
  return { isAggregated, setAggregated }
}
