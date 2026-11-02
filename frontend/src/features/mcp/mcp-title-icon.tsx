import { useTranslation } from 'react-i18next'
import { HugeiconsIcon } from '@hugeicons/react'
import { McpServerIcon } from '@hugeicons/core-free-icons'
import { Button } from '@/components/ui/button'
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '@/components/ui/tooltip'
import { requestSettings } from '@/features/settings/settings-requests'
import { useSettings } from '@/features/settings/settings-queries'
import { TitleBarActions } from '@/features/window-chrome/title-bar'
import type { MailxClient } from '@/lib/mailx-client'
import { mcpStatusText } from './mcp-status-text'
import { useMcpStatus } from './use-mcp-status'

/**
 * The MCP server's state in the title bar, only while the server is switched on.
 * A dot says running (green) or failed (red); while the server is still starting
 * there is no dot. The text carries the same state, so colour is never the only cue.
 */
export function McpTitleIcon({ client }: { client: MailxClient }) {
  const { t } = useTranslation('settings')
  const settings = useSettings(client)
  const status = useMcpStatus(client)
  if (!settings.data?.settings.mcp.enabled) return null
  const text = mcpStatusText(status, true)
  const lastClient = status?.state === 'running' ? status.lastClientName : null
  const label = `${t('mcpTitleLabel')}: ${text}`
  const dot = status?.state === 'running' ? 'bg-emerald-500' : status?.state === 'failed' ? 'bg-destructive' : null
  return (
    <TitleBarActions side="trailing" order={-2}>
      <TooltipProvider>
        <Tooltip>
          <TooltipTrigger
            render={<Button variant="ghost" size="icon-sm" aria-label={label} onClick={() => requestSettings({ section: 'mcp' })} />}
          >
            <span className="relative flex">
              <HugeiconsIcon icon={McpServerIcon} strokeWidth={2} />
              {dot && <span aria-hidden data-testid="mcp-status-dot" className={`absolute -top-0.5 -right-0.5 size-1.5 rounded-full ring-2 ring-background ${dot}`} />}
            </span>
          </TooltipTrigger>
          <TooltipContent side="bottom">
            <div className="flex flex-col gap-0.5">
              <span>{label}</span>
              {lastClient && <span>{t('mcpTitleClients', { name: lastClient })}</span>}
              <span>{t('mcpTitleOpen')}</span>
            </div>
          </TooltipContent>
        </Tooltip>
      </TooltipProvider>
    </TitleBarActions>
  )
}
