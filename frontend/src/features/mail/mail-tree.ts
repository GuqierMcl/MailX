import type { MailxClient } from '@/lib/mailx-client'

/** Accounts and their folders, cached under `mailKeys.tree()` by every reader. */
export async function loadMailTree(client: MailxClient) {
  const accounts = await client.listAccounts()
  const folders = (await Promise.all(accounts.map((account) => client.listFolders(account.id)))).flat()
  return { accounts, folders }
}
