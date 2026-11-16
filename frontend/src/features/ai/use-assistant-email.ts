import { useQuery } from '@tanstack/react-query'
import type { MailxClient } from '@/lib/mailx-client'
import { mailKeys } from '@/lib/query-client'

/**
 * The mail a tool step or a reply points to, from local data. Kept under the mail keys, so a sync round
 * refreshes it; a mail that cannot be loaded (gone, not on this device) is not retried.
 */
export function useAssistantEmail(client: MailxClient, emailId: string) {
  return useQuery({
    queryKey: [...mailKeys.all, 'assistant-email', emailId],
    queryFn: () => client.getEmail(emailId),
    staleTime: 60_000,
    retry: false,
  })
}
