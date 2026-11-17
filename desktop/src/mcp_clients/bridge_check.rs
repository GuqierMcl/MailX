//! An ephemeral stdio connection, never a mail read or a target-agent session.
use crate::mcp_sidecar::McpClientConfig;
use serde::Serialize;
use serde_json::{Value, json};
use std::{process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::Command,
};
const TIMEOUT: Duration = Duration::from_secs(10);
const MAX_REPLY: u64 = 1024 * 1024;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct McpDiagnostic {
    ok: bool,
    code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_count: Option<usize>,
}
impl McpDiagnostic {
    pub(super) fn failure(code: &'static str) -> Self {
        Self {
            ok: false,
            code,
            tool_count: None,
        }
    }
}

pub(super) async fn run(config: &McpClientConfig) -> McpDiagnostic {
    run_with_timeout(config, TIMEOUT).await
}

async fn run_with_timeout(config: &McpClientConfig, timeout: Duration) -> McpDiagnostic {
    let mut command = Command::new(&config.command);
    command
        .args(&config.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let Ok(mut child) = command.spawn() else {
        return McpDiagnostic::failure("bridgeUnavailable");
    };
    let Some(mut stdin) = child.stdin.take() else {
        let _ = child.kill().await;
        return McpDiagnostic::failure("protocolError");
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill().await;
        return McpDiagnostic::failure("protocolError");
    };
    let mut stdout = BufReader::new(stdout);
    let operation = async {
        let init = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"mailx-diagnostics","version":env!("CARGO_PKG_VERSION")}}});
        send(&mut stdin, &init).await?;
        let response = reply(&mut stdout, 1).await?;
        if response
            .get("result")
            .and_then(|v| v.get("protocolVersion"))
            .and_then(Value::as_str)
            .is_none()
        {
            return Err("protocolError");
        }
        send(
            &mut stdin,
            &json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        )
        .await?;
        send(
            &mut stdin,
            &json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}),
        )
        .await?;
        let response = reply(&mut stdout, 2).await?;
        let tools = response
            .get("result")
            .and_then(|v| v.get("tools"))
            .and_then(Value::as_array)
            .ok_or("protocolError")?;
        if tools.is_empty()
            || !tools.iter().all(|tool| {
                tool.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|name| name.starts_with("mailx_"))
            })
        {
            return Err("protocolError");
        }
        Ok(tools.len())
    };
    let result = tokio::time::timeout(timeout, operation).await;
    // Explicitly reap the child on every path, including malformed replies and timeouts.
    let _ = child.kill().await;
    let _ = child.wait().await;
    match result {
        Ok(Ok(tool_count)) => McpDiagnostic {
            ok: true,
            code: "ready",
            tool_count: Some(tool_count),
        },
        Ok(Err(code)) => McpDiagnostic::failure(code),
        Err(_) => McpDiagnostic::failure("timeout"),
    }
}

async fn send(stdin: &mut tokio::process::ChildStdin, value: &Value) -> Result<(), &'static str> {
    let mut bytes = serde_json::to_vec(value).map_err(|_| "protocolError")?;
    bytes.push(b'\n');
    stdin
        .write_all(&bytes)
        .await
        .map_err(|_| "coreUnavailable")?;
    stdin.flush().await.map_err(|_| "coreUnavailable")
}
async fn reply<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    id: u32,
) -> Result<Value, &'static str> {
    // Notifications may precede the response; bound their number and each line.
    for _ in 0..16 {
        let mut bytes = Vec::new();
        let count = (&mut *reader)
            .take(MAX_REPLY + 1)
            .read_until(b'\n', &mut bytes)
            .await
            .map_err(|_| "coreUnavailable")?;
        if count == 0 {
            return Err("coreUnavailable");
        }
        if count as u64 > MAX_REPLY || bytes.last() != Some(&b'\n') {
            return Err("protocolError");
        }
        let response: Value = serde_json::from_slice(&bytes).map_err(|_| "protocolError")?;
        if response.get("id") == Some(&json!(id)) {
            if let Some(error) = response.get("error") {
                let unavailable = error.to_string().contains("CORE_UNAVAILABLE");
                return Err(if unavailable {
                    "coreUnavailable"
                } else {
                    "protocolError"
                });
            }
            return Ok(response);
        }
        if response.get("method").is_none() {
            return Err("protocolError");
        }
    }
    Err("protocolError")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    fn fake_bridge(directory: &std::path::Path, body: &str) -> McpClientConfig {
        use std::os::unix::fs::PermissionsExt;
        let path = directory.join("fake-bridge");
        std::fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        McpClientConfig {
            command: path.to_string_lossy().into_owned(),
            args: vec![],
            display: crate::mcp_sidecar::McpClientDisplay {
                command: String::new(),
                discovery: String::new(),
            },
        }
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn process_performs_initialize_and_catalog_without_calling_tools() {
        let dir = tempfile::tempdir().unwrap();
        let config = fake_bridge(
            dir.path(),
            r#"read -r initialize
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-11-25"}}'
read -r initialized
read -r catalog
case "$catalog" in *'tools/list'*) ;; *) exit 1 ;; esac
printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"mailx_list_accounts"}]}}'
read -r never
"#,
        );
        let result = run_with_timeout(&config, Duration::from_secs(2)).await;
        assert!(result.ok);
        assert_eq!(result.tool_count, Some(1));
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn times_out_an_unresponsive_bridge() {
        let dir = tempfile::tempdir().unwrap();
        let config = fake_bridge(dir.path(), "while read -r request; do :; done\n");
        let result = run_with_timeout(&config, Duration::from_millis(50)).await;
        assert!(!result.ok);
        assert_eq!(result.code, "timeout");
    }
    #[tokio::test]
    async fn reads_notifications_then_matching_response() {
        let bytes=b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/test\"}\n{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"tools\":[]}}\n";
        assert!(
            reply(&mut BufReader::new(bytes.as_slice()), 2)
                .await
                .unwrap()
                .get("result")
                .is_some()
        );
    }
    #[tokio::test]
    async fn maps_unavailable_and_rejects_malformed_replies() {
        let bytes = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{\"data\":\"CORE_UNAVAILABLE\"}}\n";
        assert_eq!(
            reply(&mut BufReader::new(bytes.as_slice()), 1)
                .await
                .unwrap_err(),
            "coreUnavailable"
        );
        assert_eq!(
            reply(&mut BufReader::new(b"not json\n".as_slice()), 1)
                .await
                .unwrap_err(),
            "protocolError"
        );
        let bytes = vec![b'a'; MAX_REPLY as usize + 2];
        assert_eq!(
            reply(&mut BufReader::new(bytes.as_slice()), 1)
                .await
                .unwrap_err(),
            "protocolError"
        );
    }
}
