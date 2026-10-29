import { useEffect } from 'react'
import { useQueryClient, type InfiniteData } from '@tanstack/react-query'
import type { EmailPage, EmailPreviewItem } from '@/generated/accounts'
import type { MailxClient } from '@/lib/mailx-client'
import { mailKeys } from '@/lib/query-client'
import { giveUpPreview } from './preview-gave-up'

/**
 * Previews the core produced for rows near the viewport. `ready` and `failed`
 * are written into every cached list page, so no list is re-read; `missing`
 * means an attempt ended without a result and only makes the row stop
 * claiming to load, until it may be requested again.
 */
export function useEmailPreviews(client: MailxClient) {
  const queryClient = useQueryClient()
  useEffect(() => client.onEmailPreviews(({ items }) => {
    const resolved = new Map<string, EmailPreviewItem>()
    for (const item of items) {
      if (item.previewState === 'missing') giveUpPreview(item.emailId)
      else resolved.set(item.emailId, item)
    }
    if (resolved.size === 0) return
    queryClient.setQueriesData<InfiniteData<EmailPage>>({ queryKey: mailKeys.emailLists() }, (data) =>
      data && {
        ...data,
        pages: data.pages.map((page) => ({
          ...page,
          items: page.items.map((row) => {
            const item = resolved.get(row.id)
            return item ? { ...row, preview: item.preview, previewState: item.previewState } : row
          }),
        })),
      })
  }), [client, queryClient])
}
