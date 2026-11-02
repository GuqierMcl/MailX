import { t } from '@/i18n'
import type { McpStatus } from './types'

/**
 * One-line status for the settings page and the title-bar tooltip. `enabled`
 * tells "off" apart from "switched on, server not up yet".
 */
export function mcpStatusText(status: McpStatus | null, enabled: boolean): string {
  if (status?.state === 'running') return t('settings:mcpStatusRunning', { count: status.connections })
  if (status?.state === 'failed') return t('settings:mcpStatusFailed', { reason: t(`settings:mcpFailure.${status.code}`) })
  return t(enabled ? 'settings:mcpStatusStarting' : 'settings:mcpStatusOff')
}
