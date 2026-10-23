import { useQuery } from '@tanstack/react-query'
import { Alert, AlertDescription, AlertTitle } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
import { Delayed } from '@/components/ui/delayed'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select'
import { Spinner } from '@/components/ui/spinner'
import { loadMailTree } from '@/features/mail/mail-tree'
import { clientFailure, type MailxClient } from '@/lib/mailx-client'
import { mailKeys } from '@/lib/query-client'
import { SettingsGroup, SettingsPage, SettingsRow } from './settings-layout'
import { useSettings, useUpdateCompose } from './settings-queries'

/** Choices the core accepts for `compose.undoSendSeconds`. */
const UNDO_CHOICES: Record<string, string> = {
  '0': '关闭（立即发送）',
  '5': '5 秒',
  '10': '10 秒',
  '20': '20 秒',
  '30': '30 秒',
}

/** Empty value: no fixed account, so a new mail follows the mailbox being viewed. */
const FOLLOW = '__follow__'

export function ComposePane({ client }: { client: MailxClient }) {
  const settings = useSettings(client)
  const update = useUpdateCompose(client)
  const accounts = useQuery({ queryKey: mailKeys.tree(), queryFn: () => loadMailTree(client), select: (tree) => tree.accounts })
  if (settings.isPending) return <Delayed><div className="flex justify-center p-10"><Spinner className="size-5" /></div></Delayed>
  if (settings.isError) return <SettingsPage>
    <Alert variant="destructive"><AlertTitle>无法读取设置</AlertTitle><AlertDescription>{clientFailure(settings.error).message}</AlertDescription></Alert>
    <Button variant="outline" onClick={() => void settings.refetch()}>重试</Button>
  </SettingsPage>

  const compose = settings.data.settings.compose
  const list = accounts.data ?? []
  // An account that was deleted since reads as "follow"; the core also falls back to the first account.
  const accountValue = list.some((account) => account.id === compose.defaultAccountId) ? compose.defaultAccountId : FOLLOW
  const accountItems: Record<string, string> = { [FOLLOW]: '跟随当前邮箱', ...Object.fromEntries(list.map((account) => [account.id, `${account.displayName}（${account.emailAddress}）`])) }

  return <SettingsPage>
    {update.isError && <Alert variant="destructive"><AlertTitle>设置保存失败</AlertTitle><AlertDescription>{clientFailure(update.error).message}</AlertDescription></Alert>}
    <SettingsGroup title="写邮件">
      <SettingsRow label="默认发件账号" htmlFor="compose.defaultAccount"
        description="在某个邮箱的文件夹里写邮件时用那个邮箱；在“全部收件箱”等没有具体邮箱的地方，用这里选的账号。选“跟随当前邮箱”则用排在第一的账号。回复和转发总是用原邮件所属的账号。">
        <Select value={accountValue} items={accountItems}
          onValueChange={(value: string | null) => value && update.mutate({ defaultAccountId: value === FOLLOW ? '' : value })}>
          <SelectTrigger id="compose.defaultAccount" className="w-56"><SelectValue /></SelectTrigger>
          <SelectContent>
            {Object.entries(accountItems).map(([value, label]) => <SelectItem key={value} value={value}>{label}</SelectItem>)}
          </SelectContent>
        </Select>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup title="发送">
      <SettingsRow label="撤销发送" htmlFor="compose.undoSend"
        description="点“确认发送”后，邮件先等待这段时间再发出，期间可以在提示里撤销并回到写信窗口。关闭则立即发送。">
        <Select value={String(compose.undoSendSeconds)} items={UNDO_CHOICES}
          onValueChange={(value: string | null) => value && update.mutate({ undoSendSeconds: Number(value) })}>
          <SelectTrigger id="compose.undoSend" className="w-40"><SelectValue /></SelectTrigger>
          <SelectContent>
            {Object.entries(UNDO_CHOICES).map(([value, label]) => <SelectItem key={value} value={value}>{label}</SelectItem>)}
          </SelectContent>
        </Select>
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>
}
