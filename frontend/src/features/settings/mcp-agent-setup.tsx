import { useRef, useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useTranslation } from 'react-i18next'
import {
  AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent,
  AlertDialogDescription, AlertDialogFooter, AlertDialogHeader, AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import { Button } from '@/components/ui/button'
import { toast } from '@/components/ui/toast'
import type { McpAgent, McpAgentId, McpDiagnostic, McpSetupResult } from '@/features/mcp/types'
import { McpAgentIcon } from '@/features/mcp/mcp-agent-icon'
import { errorText } from '@/i18n/errors'
import type { MailxClient } from '@/lib/mailx-client'
import { SettingsGroup, SettingsRow } from './settings-layout'
import { McpFeedback } from './mcp-feedback'

const agentsKey = ['mcp', 'agents'] as const
type AgentFeedback = { kind: 'outcome'; state: McpSetupResult['state'] } | { kind: 'error'; cause: unknown }

/** Configuration state stays separate from the self-reported live session list. */
export function McpAgentSetup({ client }: { client: MailxClient }) {
  const { t } = useTranslation('settings')
  const queryClient = useQueryClient()
  const agents = useQuery({ queryKey: agentsKey, queryFn: () => client.getMcpAgents(), retry: false })
  const [pending, setPending] = useState<McpAgentId | 'diagnose' | null>(null)
  const busy = useRef(false)
  const [conflict, setConflict] = useState<McpAgent | null>(null)
  const [feedback, setFeedback] = useState<Partial<Record<McpAgentId, AgentFeedback>>>({})
  const [diagnostic, setDiagnostic] = useState<McpDiagnostic | null>(null)
  const [diagnosticError, setDiagnosticError] = useState<unknown>(null)

  const run = async (agent: McpAgent, action: 'setup' | 'remove', replace = false) => {
    if (busy.current) return
    busy.current = true
    setConflict(null)
    setPending(agent.id)
    setFeedback((current) => ({ ...current, [agent.id]: undefined }))
    try {
      await queryClient.cancelQueries({ queryKey: agentsKey })
      const result = action === 'setup'
        ? await client.setupMcpAgent(agent.id, replace)
        : await client.removeMcpAgent(agent.id)
      queryClient.setQueryData<McpAgent[]>(agentsKey, (current) => current?.map((item) => item.id === agent.id ? result.agent : item))
      setFeedback((current) => ({ ...current, [agent.id]: { kind: 'outcome', state: result.state } }))
      if (result.state === 'conflict' && action === 'setup') setConflict(result.agent)
    } catch (cause) {
      setFeedback((current) => ({ ...current, [agent.id]: { kind: 'error', cause } }))
    } finally {
      busy.current = false
      setPending(null)
    }
  }

  const connect = (agent: McpAgent) => {
    if (agent.configuration === 'conflict') setConflict(agent)
    else void run(agent, 'setup')
  }

  const copy = async (agent: McpAgent) => {
    try {
      await navigator.clipboard.writeText(agent.snippet)
      toast.success(t('mcpAgentCopied', { name: agent.name }))
    } catch (cause) {
      toast.error(t('mcpCopyFailed'), { description: errorText(cause) })
    }
  }

  const diagnose = async () => {
    if (busy.current) return
    busy.current = true
    setPending('diagnose')
    setDiagnostic(null)
    setDiagnosticError(null)
    try {
      setDiagnostic(await client.diagnoseMcp())
    } catch (cause) {
      setDiagnosticError(cause)
    } finally {
      busy.current = false
      setPending(null)
    }
  }

  const openDocs = async (agent: McpAgent) => {
    try {
      await client.openExternalLink(agent.docsUrl)
    } catch (cause) {
      toast.error(t('mcpActionFailed'), { description: errorText(cause) })
    }
  }

  return <>
    <SettingsGroup title={t('mcpAgents')} description={t('mcpAgentsHint')}>
      {agents.isError && <McpFeedback tone="error">{errorText(agents.error)}</McpFeedback>}
      {agents.isPending && <p className="text-sm text-muted-foreground">{t('mcpAgentsLoading')}</p>}
      {agents.data?.map((agent) => <SettingsRow key={agent.id} label={<span className="flex items-center gap-2">
        <McpAgentIcon id={agent.id} />{agent.name}
      </span>} description={<span className="flex flex-col gap-1">
        {feedback[agent.id]
          ? <AgentResult feedback={feedback[agent.id]!} />
          : <>
            <span>{t(`mcpAgentState.${agent.configuration}`)}</span>
            <span>{t(agent.method === 'manual' ? agent.id === 'deepseekHarness' ? 'mcpAgentHarnessHint' : 'mcpAgentManualHint' : agent.method === 'deeplink' ? 'mcpAgentDeeplinkHint' : !agent.available ? 'mcpAgentUnavailableHint' : 'mcpAgentLocalHint', { name: agent.name })}</span>
          </>}
      </span>}>
        {agent.method !== 'manual' && <Button variant="outline" size="sm" disabled={pending !== null || !agent.available}
          aria-label={t('mcpAgentConnectLabel', { name: agent.name })} onClick={() => connect(agent)}>
          {pending === agent.id ? t('mcpAgentWorking') : t(agent.configuration === 'configured' ? 'mcpAgentReconnect' : agent.configuration === 'conflict' ? 'mcpAgentRepair' : 'mcpAgentConnect')}
        </Button>}
        <Button variant="ghost" size="sm" aria-label={t('mcpAgentCopyLabel', { name: agent.name })} onClick={() => void copy(agent)}>{t('mcpCopyShort')}</Button>
        {agent.configuration === 'configured' && <Button variant="ghost" size="sm" disabled={pending !== null}
          aria-label={t('mcpAgentRemoveLabel', { name: agent.name })} onClick={() => void run(agent, 'remove')}>{t('mcpAgentRemove')}</Button>}
        {agent.method === 'manual' && <Button variant="ghost" size="sm" aria-label={t('mcpAgentDocsLabel', { name: agent.name })}
          onClick={() => void openDocs(agent)}>{t('mcpAgentDocs')}</Button>}
      </SettingsRow>)}
      <div>
        <Button variant="outline" size="sm" disabled={pending !== null || agents.isFetching} onClick={() => { setFeedback({}); void agents.refetch() }}>{t('mcpAgentsRefresh')}</Button>
      </div>
    </SettingsGroup>
    <SettingsGroup title={t('mcpDiagnostics')} description={t('mcpDiagnosticsHint')}>
      <div><Button variant="outline" size="sm" disabled={pending !== null} onClick={() => void diagnose()}>
        {pending === 'diagnose' ? t('mcpDiagnosing') : t('mcpDiagnose')}
      </Button></div>
      {diagnostic && <McpFeedback tone={diagnostic.ok ? 'success' : diagnostic.code === 'disabled' ? 'neutral' : 'error'}>
        {t(`mcpDiagnostic.${diagnostic.code}`)}
      </McpFeedback>}
      {diagnosticError !== null && <McpFeedback tone="error">{errorText(diagnosticError)}</McpFeedback>}
    </SettingsGroup>
    <AlertDialog open={conflict !== null} onOpenChange={(open) => { if (!open) setConflict(null) }}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t('mcpConflictTitle', { name: conflict?.name ?? '' })}</AlertDialogTitle>
          <AlertDialogDescription>{t('mcpConflictDescription')}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel>{t('mcpConfirmCancel')}</AlertDialogCancel>
          <AlertDialogAction onClick={() => { if (conflict) void run(conflict, 'setup', true) }}>{t('mcpConflictReplace')}</AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  </>
}

function AgentResult({ feedback }: { feedback: AgentFeedback }) {
  const { t } = useTranslation('settings')
  if (feedback.kind === 'error') return <McpFeedback tone="error">
    <span className="block">{t('mcpActionFailed')}</span>
    {errorText(feedback.cause)}
  </McpFeedback>
  const tone = feedback.state === 'unavailable' || feedback.state === 'conflict' ? 'error'
    : feedback.state === 'handoff' ? 'neutral' : 'success'
  return <McpFeedback tone={tone}>{t(`mcpSetupOutcome.${feedback.state}`)}</McpFeedback>
}
