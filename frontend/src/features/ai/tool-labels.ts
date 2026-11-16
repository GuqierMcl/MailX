import { t } from '@/i18n'
import type { RunTool } from './use-assistant-run'

/** How long "撤销" stays on a finished organize step. */
export const UNDO_WINDOW_MS = 60_000
/** Most email chips listed under one step. */
export const CHIP_LIMIT = 5
type CountedTool = 'search_emails' | 'read_email' | 'archive_emails' | 'unarchive_emails' | 'set_email_flags'
type PlainTool = 'list_accounts' | 'list_folders' | 'list_drafts' | 'read_draft' | 'list_outbox' | 'get_outbox' | 'refresh_sync'
  | 'create_draft' | 'create_reply_draft' | 'update_draft' | 'delete_draft' | 'add_draft_attachment' | 'remove_draft_attachment'
  | 'send_email' | 'cancel_outbox' | 'other'
const COUNTED: ReadonlySet<string> = new Set<CountedTool>(['search_emails', 'read_email', 'archive_emails', 'unarchive_emails', 'set_email_flags'])
const PLAIN: ReadonlySet<string> = new Set<PlainTool>([
  'list_accounts', 'list_folders', 'list_drafts', 'read_draft', 'list_outbox', 'get_outbox', 'refresh_sync', 'create_draft', 'create_reply_draft',
  'update_draft', 'delete_draft', 'add_draft_attachment', 'remove_draft_attachment', 'send_email', 'cancel_outbox',
])

/** The step in words: "正在搜索邮件…" while it runs, "搜索了 12 封邮件" after. */
export function toolLabel(tool: RunTool): string {
  if (tool.status === 'failed') return t('ai:chat.tool.failed')
  if (tool.status === 'denied') return t('ai:chat.tool.denied')
  const name = tool.name
  const { summary } = tool
  if (COUNTED.has(name)) {
    const counted = name as CountedTool
    if (tool.status === 'running') return t(`ai:chat.tool.${counted}.running`)
    const count = summary.type === 'found' || summary.type === 'changed' ? summary.count : summary.type === 'read' ? summary.emailIds.length : 0
    return t(`ai:chat.tool.${counted}.done`, { count })
  }
  const plain = (PLAIN.has(name) ? name : 'other') as PlainTool
  return t(tool.status === 'running' ? `ai:chat.tool.${plain}.running` : `ai:chat.tool.${plain}.done`)
}
