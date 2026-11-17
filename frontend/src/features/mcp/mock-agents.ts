import type { McpAgent, McpClientConfig } from './types'
import { mcpClientSnippet } from './client-snippet'

/** Synthetic adapters for the browser preview and UI tests; never touch host files. */
export function mockMcpAgents(config: McpClientConfig): McpAgent[] {
  const snippet = mcpClientSnippet(config)
  return [
    { id: 'claudeCode', name: 'Claude Code', method: 'config', docsUrl: 'https://code.claude.com/docs/en/mcp', snippet },
    { id: 'codex', name: 'Codex', method: 'config', docsUrl: 'https://developers.openai.com/codex/mcp/', snippet: `[mcp_servers.mailx]\ncommand = ${JSON.stringify(config.command)}\nargs = ${JSON.stringify(config.args)}\n` },
    { id: 'cursor', name: 'Cursor', method: 'deeplink', docsUrl: 'https://cursor.com/docs/context/mcp', snippet },
    { id: 'openCode', name: 'OpenCode', method: 'config', docsUrl: 'https://opencode.ai/docs/mcp-servers/', snippet: JSON.stringify({ mcp: { servers: { mailx: { type: 'local', command: [config.command, ...config.args] } } } }, null, 2) },
    { id: 'trae', name: 'Trae', method: 'deeplink', docsUrl: 'https://docs.trae.ai/ide/mcp-server-install-links', snippet },
    { id: 'deepseekHarness', name: 'DeepSeek Harness', method: 'manual', docsUrl: 'https://github.com/deepseek-ai/deepseek-harness', snippet: `- insert:\n    - id: mailx-mcp\n      name: '@deepseek-ai/dsh-mcp-client'\n      config:\n        serverName: mailx\n        transport: stdio\n        command: ${JSON.stringify(config.command)}\n        args: ${JSON.stringify(config.args)}\n` },
  ].map((agent) => ({ ...agent, configuration: agent.method === 'manual' ? 'manual' : 'missing', available: agent.method !== 'manual' }) as McpAgent)
}
