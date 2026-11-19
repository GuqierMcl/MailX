import { t } from '@/i18n'
import type { RunTool } from './use-assistant-run'

/** How long "撤销" stays on a finished organize step. */
export const UNDO_WINDOW_MS = 60_000
/** Most email chips listed under one step. */
export const CHIP_LIMIT = 5
type CountedTool = 'mail_search' | 'mail_read' | 'mail_conversation' | 'mail_contacts' | 'mail_archive' | 'mail_flags'
type PlainTool = 'accounts_list' | 'mail_folders' | 'draft_list' | 'draft_read' | 'outbox_read' | 'accounts_refresh_sync'
  | 'draft_create' | 'draft_reply' | 'draft_update' | 'draft_delete' | 'draft_attachments'
  | 'outbox_send' | 'outbox_cancel' | 'show_in_app' | 'mail_save_attachment' | 'mail_help' | 'other'
const COUNTED: ReadonlySet<string> = new Set<CountedTool>(['mail_search', 'mail_read', 'mail_conversation', 'mail_contacts', 'mail_archive', 'mail_flags'])
const PLAIN: ReadonlySet<string> = new Set<PlainTool>([
  'accounts_list', 'mail_folders', 'draft_list', 'draft_read', 'outbox_read', 'accounts_refresh_sync', 'draft_create', 'draft_reply',
  'draft_update', 'draft_delete', 'draft_attachments', 'outbox_send', 'outbox_cancel', 'show_in_app', 'mail_save_attachment', 'mail_help',
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
