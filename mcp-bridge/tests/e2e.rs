//! Runs the built `mailx-mcp` binary against a real server through stdio.

#[path = "../../mcp-server/tests/common/mod.rs"]
mod common;

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use common::Fixture;
use mailx_mcp_server::{McpServer, discovery_path};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

const BIN: &str = env!("CARGO_BIN_EXE_mailx-mcp");

struct Bridge {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: tokio::io::Lines<BufReader<ChildStdout>>,
}

fn spawn_bridge(discovery: &Path) -> Bridge {
    let mut child = Command::new(BIN)
        .arg("--discovery")
        .arg(discovery)
        .env_remove("MAILX_MCP_DISCOVERY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let stdin = child.stdin.take();
    let stdout = BufReader::new(child.stdout.take().unwrap()).lines();
    Bridge {
        child,
        stdin,
        stdout,
    }
}

impl Bridge {
    async fn send(&mut self, message: Value) {
        let stdin = self.stdin.as_mut().unwrap();
        stdin
            .write_all(format!("{message}\n").as_bytes())
            .await
            .unwrap();
        stdin.flush().await.unwrap();
    }

    async fn recv(&mut self) -> Value {
        let line = tokio::time::timeout(Duration::from_secs(10), self.stdout.next_line())
            .await
            .expect("bridge reply timed out")
            .unwrap()
            .expect("bridge closed stdout");
        serde_json::from_str(&line).unwrap()
    }

    async fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await;
        self.recv().await
    }

    async fn initialize(&mut self) -> Value {
        let reply = self
            .request(
                1,
                "initialize",
                json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": {"name": "bridge-e2e", "version": "0"}
                }),
            )
            .await;
        self.send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .await;
        reply
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn bridge_relays_mcp_between_stdio_and_the_server() {
    let fixture = Fixture::new().await;
    let handle = McpServer::start(fixture.config()).await.unwrap();
    let mut bridge = spawn_bridge(&discovery_path(fixture.data_dir()));

    let init = bridge.initialize().await;
    assert_eq!(init["result"]["serverInfo"]["name"], "mailx");

    let list = bridge.request(2, "tools/list", json!({})).await;
    let tools = list["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 4);
    assert!(
        tools
            .iter()
            .all(|tool| tool["name"].as_str().unwrap().starts_with("mailx_"))
    );

    let accounts = bridge
        .request(
            3,
            "tools/call",
            json!({"name": "mailx_list_accounts", "arguments": {}}),
        )
        .await;
    assert_ne!(accounts["result"]["isError"], json!(true));
    assert!(
        accounts["result"]["structuredContent"]
            .to_string()
            .contains(&fixture.account_id)
    );

    assert!(matches!(
        handle.status(),
        mailx_mcp_server::McpStatus::Running { ref clients } if clients.len() == 1
    ));
    // Closing stdin ends the bridge cleanly.
    drop(bridge.stdin.take());
    let status = tokio::time::timeout(Duration::from_secs(10), bridge.child.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(status.success(), "{status:?}");
    // The server frees the session once the bridge's connection drops.
    let mut freed = false;
    for _ in 0..50 {
        if matches!(
            handle.status(),
            mailx_mcp_server::McpStatus::Running { ref clients } if clients.is_empty()
        ) {
            freed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(freed, "session slot must be freed after the bridge exits");
    handle.stop().await;
}

async fn assert_core_unavailable(mut bridge: Bridge, reason: &str) {
    // A notification first: it must not be answered.
    bridge
        .send(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
        .await;
    let reply = bridge
        .request(
            6,
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "x", "version": "0"}}),
        )
        .await;
    assert_eq!(reply["id"], 6);
    assert_eq!(reply["error"]["code"], -32001);
    assert_eq!(reply["error"]["data"]["code"], "CORE_UNAVAILABLE");
    assert_eq!(reply["error"]["data"]["reason"], reason);
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap()
            .contains("MailX")
    );
    let status = tokio::time::timeout(Duration::from_secs(10), bridge.child.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(status.code(), Some(69));
}

#[tokio::test(flavor = "multi_thread")]
async fn missing_discovery_file_answers_core_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let bridge = spawn_bridge(&dir.path().join("mcp-discovery.json"));
    assert_core_unavailable(bridge, "discoveryMissing").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn stale_discovery_file_answers_core_unavailable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mcp-discovery.json");
    #[cfg(not(windows))]
    let endpoint = dir.path().join("gone.sock");
    #[cfg(windows)]
    let endpoint = std::path::PathBuf::from(r"\\.\pipe\mailx-mcp-does-not-exist");
    std::fs::write(
        &path,
        json!({"version": 1, "endpoint": endpoint, "token": "t", "pid": 1, "instanceId": "i"})
            .to_string(),
    )
    .unwrap();
    assert_core_unavailable(spawn_bridge(&path), "connectFailed").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn wrong_token_answers_core_unavailable() {
    let fixture = Fixture::new().await;
    let handle = McpServer::start(fixture.config()).await.unwrap();
    let path = discovery_path(fixture.data_dir());
    let mut discovery: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    discovery["token"] = json!("0".repeat(64));
    let forged = fixture.data_dir().join("forged.json");
    std::fs::write(&forged, discovery.to_string()).unwrap();

    assert_core_unavailable(spawn_bridge(&forged), "handshakeRejected").await;
    handle.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn disabled_server_makes_the_bridge_unavailable() {
    let fixture = Fixture::new().await;
    let handle = McpServer::start(fixture.config()).await.unwrap();
    let path = discovery_path(fixture.data_dir());
    let saved = fixture.data_dir().join("saved.json");
    std::fs::copy(&path, &saved).unwrap();
    handle.stop().await;
    // The old token and endpoint no longer work.
    assert_core_unavailable(spawn_bridge(&saved), "connectFailed").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn bridge_exits_when_the_server_closes_first_even_with_stdin_open() {
    let fixture = Fixture::new().await;
    let handle = McpServer::start(fixture.config()).await.unwrap();
    let mut bridge = spawn_bridge(&discovery_path(fixture.data_dir()));
    bridge.initialize().await;

    // stdin stays open (held by `bridge.stdin`); the server goes away.
    handle.stop().await;
    let status = tokio::time::timeout(Duration::from_secs(5), bridge.child.wait())
        .await
        .expect("bridge must exit after the server closes")
        .unwrap();
    assert!(status.success(), "{status:?}");
    drop(bridge.stdin.take());
}
