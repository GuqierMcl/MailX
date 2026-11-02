/**
 * Local MCP server status, mirroring `McpStatus` in `mcp-server/src/status.rs`
 * (serialized by the desktop host). Hand-written: the type lives outside
 * `mailx-core`, so ts-rs does not generate it.
 */
export type McpFailureCode =
  | 'endpointInUse'
  | 'endpointCreateFailed'
  | 'permissionDenied'
  | 'discoveryWriteFailed'
  | 'tokenUnavailable'
  | 'acceptFailed'

export type McpStatus =
  | { state: 'off' }
  | { state: 'running'; connections: number; lastClientName: string | null }
  | { state: 'failed'; code: McpFailureCode }

/**
 * What a client's `mcpServers` entry runs, mirroring `McpClientConfig` in
 * `desktop/src/mcp_sidecar.rs`. `args` always begin with `--discovery <file>`.
 */
export type McpClientConfig = { command: string; args: string[] }
