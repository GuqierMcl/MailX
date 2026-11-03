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

/** One open session; `label` is self-reported (title, else name), null until it identifies itself. */
export type McpClient = { label: string | null }

export type McpStatus =
  | { state: 'off' }
  | { state: 'running'; clients: McpClient[] }
  | { state: 'failed'; code: McpFailureCode }

/**
 * What a client's `mcpServers` entry runs, mirroring `McpClientConfig` in
 * `desktop/src/mcp_sidecar.rs`. `args` always begin with `--discovery <file>`.
 */
export type McpClientConfig = {
  command: string
  args: string[]
  /** The same paths for display only, with the home directory written as `~`. Never copy these. */
  display: { command: string; discovery: string }
}
