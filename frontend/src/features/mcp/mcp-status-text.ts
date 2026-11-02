import { t } from '@/i18n'
import type { McpClient, McpStatus } from './types'

/**
 * One-line status for the settings page and the title-bar tooltip. `enabled`
 * tells "off" apart from "switched on, server not up yet".
 */
export function mcpStatusText(status: McpStatus | null, enabled: boolean): string {
  if (status?.state === 'running') return t('settings:mcpStatusRunning', { count: status.clients.length })
  if (status?.state === 'failed') return t('settings:mcpStatusFailed', { reason: t(`settings:mcpFailure.${status.code}`) })
  return t(enabled ? 'settings:mcpStatusStarting' : 'settings:mcpStatusOff')
}

/**
 * The connected clients, one entry per distinct label in first-connect order;
 * repeats merge as "Claude Code ×2" and unidentified sessions as "Unknown client".
 */
export function mcpClientGroups(clients: McpClient[]): string[] {
  const counts = new Map<string, number>()
  for (const client of clients) {
    const label = client.label ?? t('settings:mcpClientUnknown')
    counts.set(label, (counts.get(label) ?? 0) + 1)
  }
  return [...counts].map(([label, count]) => (count > 1 ? t('settings:mcpClientGroup', { label, count }) : label))
}
