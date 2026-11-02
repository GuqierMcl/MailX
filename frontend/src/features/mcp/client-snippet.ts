import type { McpClientConfig } from './types'

/** The snippet clients paste into their `mcpServers` configuration. */
export function mcpClientSnippet(config: McpClientConfig): string {
  return JSON.stringify({ mcpServers: { mailx: { command: config.command, args: config.args } } }, null, 2)
}
