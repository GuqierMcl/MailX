//! End-to-end tests over a real Unix socket with an rmcp client.
#![cfg(unix)]

mod common;

use std::os::unix::fs::PermissionsExt as _;
use std::time::Duration;

use common::Fixture;
use mailx_mcp_server::{
    Discovery, MAX_CONNECTIONS, MAX_MESSAGE_BYTES, McpServer, McpStatus, discovery_path,
};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, ClientConfig, Implementation,
    JsonObject,
};
use rmcp::service::{RoleClient, RunningService};
use rmcp::transport::async_rw::AsyncRwTransport;
use rmcp::{ErrorData, ServiceError, ServiceExt as _};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::UnixStream;

fn read_discovery(fixture: &Fixture) -> Discovery {
    let bytes = std::fs::read(discovery_path(fixture.data_dir())).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn raw_handshake(endpoint: &str, line: &[u8]) -> (UnixStream, String) {
    let mut stream = UnixStream::connect(endpoint).await.unwrap();
    stream.write_all(line).await.unwrap();
    let mut reply = String::new();
    let mut reader = BufReader::new(&mut stream);
    tokio::time::timeout(Duration::from_secs(3), reader.read_line(&mut reply))
        .await
        .unwrap()
        .unwrap();
    (stream, reply)
}

fn labels(clients: &[mailx_mcp_server::McpClient]) -> Vec<Option<&str>> {
    clients
        .iter()
        .map(|client| client.label.as_deref())
        .collect()
}

async fn connect(discovery: &Discovery, name: &str) -> RunningService<RoleClient, ClientConfig> {
    connect_as(discovery, Implementation::new(name, "0.0.0")).await
}

async fn connect_as(
    discovery: &Discovery,
    info: Implementation,
) -> RunningService<RoleClient, ClientConfig> {
    let hello = format!("{{\"bridge\":1,\"token\":\"{}\"}}\n", discovery.token);
    let (stream, reply) = raw_handshake(&discovery.endpoint, hello.as_bytes()).await;
    assert_eq!(reply.trim(), "{\"ok\":true}");
    let (read, write) = tokio::io::split(stream);
    ClientConfig::new(ClientCapabilities::default(), info)
        .serve(AsyncRwTransport::new_client(read, write))
        .await
        .unwrap()
}

/// The server closed the connection: EOF, or a reset when it closed with
/// unread bytes still in its receive buffer (Linux sends RST then).
async fn assert_closed(stream: &mut UnixStream) {
    let mut rest = Vec::new();
    match stream.read_to_end(&mut rest).await {
        Ok(read) => assert_eq!(read, 0, "no data expected after the rejection"),
        Err(error) => assert!(
            matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::BrokenPipe
            ),
            "{error:?}"
        ),
    }
}

fn args(value: Value) -> JsonObject {
    value.as_object().cloned().unwrap()
}

async fn call(
    client: &RunningService<RoleClient, ClientConfig>,
    name: &str,
    arguments: Value,
) -> Result<CallToolResult, ServiceError> {
    client
        .peer()
        .call_tool(CallToolRequestParams::new(name.to_owned()).with_arguments(args(arguments)))
        .await
}

fn rpc_code(error: &ServiceError) -> Option<i32> {
    match error {
        ServiceError::McpError(ErrorData { code, .. }) => Some(code.0),
        _ => None,
    }
}

#[tokio::test]
async fn tools_list_and_calls_work_end_to_end() {
    let fixture = Fixture::new().await;
    let handle = McpServer::start(fixture.config()).await.unwrap();
    let client = connect(&read_discovery(&fixture), "e2e-client").await;

    let tools = client.peer().list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 22);
    for tool in &tools {
        assert!(tool.name.starts_with("mailx_"), "{}", tool.name);
        assert!(tool.output_schema.is_some(), "{}", tool.name);
        assert!(tool.annotations.is_some());
        let meta = serde_json::to_value(tool).unwrap();
        assert!(meta["_meta"]["mailx/requiredActions"].is_array());
        assert!(meta["_meta"]["mailx/limits"].is_object());
    }

    for tool in &tools {
        let hint = tool.annotations.as_ref().unwrap().destructive_hint;
        assert_eq!(
            hint,
            Some(matches!(
                tool.name.as_ref(),
                "mailx_delete_draft" | "mailx_send_email" | "mailx_remove_draft_attachment"
            )),
            "{}",
            tool.name
        );
    }

    let accounts = call(&client, "mailx_list_accounts", json!({}))
        .await
        .unwrap();
    assert_ne!(accounts.is_error, Some(true));
    let structured = accounts.structured_content.unwrap();
    assert!(structured.to_string().contains(&fixture.account_id));

    let search = call(&client, "mailx_search_emails", json!({"query": "hello"}))
        .await
        .unwrap();
    assert_ne!(search.is_error, Some(true));
    assert!(search.structured_content.is_some());

    // Status reflects the client.
    assert!(matches!(
        handle.status(),
        McpStatus::Running { ref clients } if labels(clients) == [Some("e2e-client")]
    ));
    client.cancel().await.unwrap();
    handle.stop().await;
}

#[tokio::test]
async fn default_grant_exposes_organizing_and_reports_safe_per_field_failures() {
    let fixture = Fixture::new().await;
    let handle = McpServer::start(fixture.config()).await.unwrap();
    let client = connect(&read_discovery(&fixture), "organize-client").await;
    let tools = client.peer().list_all_tools().await.unwrap();
    for name in [
        "mailx_set_email_flags",
        "mailx_archive_emails",
        "mailx_unarchive_emails",
        "mailx_refresh_sync",
        "mailx_show_draft_in_app",
    ] {
        assert!(tools.iter().any(|tool| tool.name == name), "{name}");
    }
    for name in [
        "mailx_delete_emails",
        "mailx_send_draft",
        "mailx_get_grant_scope",
    ] {
        assert!(!tools.iter().any(|tool| tool.name == name), "{name}");
    }
    let flags = tools
        .iter()
        .find(|t| t.name == "mailx_set_email_flags")
        .unwrap();
    let flags = serde_json::to_value(flags).unwrap();
    assert_eq!(flags["annotations"]["readOnlyHint"], false);
    assert_eq!(flags["annotations"]["idempotentHint"], true);
    assert_eq!(flags["annotations"]["openWorldHint"], true);
    assert_eq!(flags["_meta"]["mailx/writesServerData"], true);
    let archive = tools
        .iter()
        .find(|t| t.name == "mailx_archive_emails")
        .unwrap();
    let archive = serde_json::to_value(archive).unwrap();
    assert_eq!(archive["annotations"]["idempotentHint"], true);
    assert_eq!(archive["annotations"]["openWorldHint"], false);
    assert_eq!(archive["_meta"]["mailx/writesServerData"], false);
    let read = tools.iter().find(|t| t.name == "mailx_read_email").unwrap();
    let read = serde_json::to_value(read).unwrap();
    assert_eq!(read["annotations"]["readOnlyHint"], true);
    assert_eq!(read["annotations"]["openWorldHint"], true);
    assert_eq!(read["_meta"]["mailx/writesLocalData"], true);
    assert_eq!(read["_meta"]["mailx/writesServerData"], false);

    let missing = "00000000-0000-4000-8000-000000000099";
    let flags = call(
        &client,
        "mailx_set_email_flags",
        json!({
            "emailIds": [missing], "isRead": true, "isStarred": true,
        }),
    )
    .await
    .unwrap();
    assert_ne!(flags.is_error, Some(true));
    let result = flags.structured_content.unwrap();
    for field in ["isRead", "isStarred"] {
        assert_eq!(result["results"][0][field]["status"], "failed");
        assert_eq!(result["results"][0][field]["errorCode"], "notFound");
    }
    for name in ["mailx_archive_emails", "mailx_unarchive_emails"] {
        let result = call(&client, name, json!({"emailIds": [missing]}))
            .await
            .unwrap();
        assert_eq!(result.is_error, Some(true));
        assert!(result.structured_content.is_none());
        let text = serde_json::to_value(result.content).unwrap().to_string();
        assert!(text.contains("notFound"), "{text}");
        assert!(!text.contains("notPermitted"), "{text}");
    }
    let sync = call(
        &client,
        "mailx_refresh_sync",
        json!({"accountId": fixture.account_id}),
    )
    .await
    .unwrap();
    assert_ne!(sync.is_error, Some(true));
    assert!(sync.structured_content.unwrap()["accepted"].is_boolean());
    let draft = call(
        &client,
        "mailx_show_draft_in_app",
        json!({"draftId": missing}),
    )
    .await
    .unwrap();
    assert_eq!(draft.is_error, Some(true));
    assert!(
        serde_json::to_value(draft.content)
            .unwrap()
            .to_string()
            .contains("notFound")
    );
    let search = call(
        &client,
        "mailx_search_emails",
        json!({"starred": true, "archive": "only"}),
    )
    .await
    .unwrap();
    assert_ne!(search.is_error, Some(true));
    assert!(search.structured_content.unwrap()["coverage"].is_object());

    client.cancel().await.unwrap();
    handle.stop().await;
}

#[tokio::test]
async fn sending_and_attachments_work_through_the_default_mcp_grant() {
    let fixture = Fixture::new().await;
    let handle = McpServer::start(fixture.config()).await.unwrap();
    let client = connect(&read_discovery(&fixture), "sending-client").await;
    let tools = client.peer().list_all_tools().await.unwrap();
    for name in [
        "mailx_send_email",
        "mailx_list_outbox",
        "mailx_get_outbox",
        "mailx_cancel_outbox",
        "mailx_add_draft_attachment",
        "mailx_remove_draft_attachment",
    ] {
        assert!(tools.iter().any(|tool| tool.name == name), "{name}");
    }
    let send = serde_json::to_value(
        tools
            .iter()
            .find(|tool| tool.name == "mailx_send_email")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(send["annotations"]["readOnlyHint"], false);
    assert_eq!(send["annotations"]["destructiveHint"], true);
    assert_eq!(send["annotations"]["openWorldHint"], true);
    assert_eq!(send["_meta"]["mailx/writesServerData"], true);
    assert_eq!(send["_meta"]["mailx/requiredActions"], json!(["mail:send"]));

    let created = call(
        &client,
        "mailx_create_draft",
        json!({"to": [{"address": "recipient@example.test"}], "subject": "MCP send test", "bodyHtml": "<p>Local fixture only</p>"}),
    )
    .await
    .unwrap();
    assert_ne!(created.is_error, Some(true));
    let draft_id = created.structured_content.unwrap()["draftId"].clone();
    let read = call(&client, "mailx_read_draft", json!({"draftId": draft_id}))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    let path = fixture.dir.path().join("attachment.txt");
    std::fs::write(&path, "synthetic attachment").unwrap();
    let added = call(
        &client,
        "mailx_add_draft_attachment",
        json!({"draftId": draft_id, "expectedRevision": read["revision"], "filePath": path}),
    )
    .await
    .unwrap();
    assert_ne!(added.is_error, Some(true), "{:?}", added.content);
    let added = added.structured_content.unwrap();
    assert_eq!(added["attachment"]["name"], "attachment.txt");
    assert_eq!(added["attachment"]["origin"], "mcp");
    assert!(!added.to_string().contains(path.to_str().unwrap()));
    let read = call(&client, "mailx_read_draft", json!({"draftId": draft_id}))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(read["attachments"][0]["id"], added["attachment"]["id"]);
    let removed = call(
        &client,
        "mailx_remove_draft_attachment",
        json!({"draftId": draft_id, "expectedRevision": added["revision"], "attachmentId": added["attachment"]["id"]}),
    )
    .await
    .unwrap();
    assert_ne!(removed.is_error, Some(true), "{:?}", removed.content);
    let removed = removed.structured_content.unwrap();
    assert_eq!(removed["removed"], true);

    let input = json!({
        "action": "send", "draftId": draft_id, "expectedRevision": removed["revision"],
        "clientMutationId": "00000000-0000-4000-8000-000000000057"
    });
    let queued = call(&client, "mailx_send_email", input.clone())
        .await
        .unwrap();
    assert_ne!(queued.is_error, Some(true), "{:?}", queued.content);
    let queued = queued.structured_content.unwrap();
    assert_eq!(queued["state"], "queued");
    let outbox_id = queued["id"].clone();
    let replay = call(&client, "mailx_send_email", input).await.unwrap();
    assert_eq!(replay.structured_content.unwrap()["id"], outbox_id);

    let retry = call(
        &client,
        "mailx_send_email",
        json!({"action": "retry", "outboxId": outbox_id}),
    )
    .await
    .unwrap();
    assert_ne!(retry.is_error, Some(true));
    let retry = retry.structured_content.unwrap();
    assert_eq!(retry["id"], outbox_id);
    assert_eq!(retry["state"], "queued");
    assert_eq!(retry["notBefore"], queued["notBefore"]);
    let list = call(
        &client,
        "mailx_list_outbox",
        json!({"accountId": fixture.account_id, "states": ["queued"], "limit": 1}),
    )
    .await
    .unwrap()
    .structured_content
    .unwrap();
    assert_eq!(list["items"][0]["id"], outbox_id);
    let item = call(&client, "mailx_get_outbox", json!({"outboxId": outbox_id}))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(item["state"], "queued");
    assert!(item.get("serverReply").is_none());

    let invalid = call(
        &client,
        "mailx_send_email",
        json!({"action": "retry", "outboxId": outbox_id, "draftId": draft_id}),
    )
    .await
    .unwrap();
    assert_eq!(invalid.is_error, Some(true));
    assert!(
        serde_json::to_value(invalid.content)
            .unwrap()
            .to_string()
            .contains("invalidInput")
    );
    let canceled = call(
        &client,
        "mailx_cancel_outbox",
        json!({"outboxId": outbox_id}),
    )
    .await
    .unwrap();
    assert_ne!(canceled.is_error, Some(true));
    assert_eq!(canceled.structured_content.unwrap()["state"], "canceled");
    let draft = call(&client, "mailx_read_draft", json!({"draftId": draft_id}))
        .await
        .unwrap()
        .structured_content
        .unwrap();
    assert_eq!(draft["locked"], false);
    assert_eq!(draft["attachments"], json!([]));

    client.cancel().await.unwrap();
    handle.stop().await;
}

#[tokio::test]
async fn invalid_input_and_unknown_tools_are_tool_errors_but_unprefixed_is_a_protocol_error() {
    let fixture = Fixture::new().await;
    let handle = McpServer::start(fixture.config()).await.unwrap();
    let client = connect(&read_discovery(&fixture), "e2e-client").await;

    let invalid = call(&client, "mailx_search_emails", json!({"query": 5}))
        .await
        .unwrap();
    assert_eq!(invalid.is_error, Some(true));
    assert!(invalid.structured_content.is_none());
    let text = serde_json::to_value(&invalid.content).unwrap().to_string();
    assert!(text.contains("invalidInput"), "{text}");

    let unknown = call(&client, "mailx_nonexistent", json!({})).await.unwrap();
    assert_eq!(unknown.is_error, Some(true));
    assert!(unknown.structured_content.is_none());
    let text = serde_json::to_value(&unknown.content).unwrap().to_string();
    assert!(text.contains("unknownTool"), "{text}");

    let unprefixed = call(&client, "list_accounts", json!({})).await.unwrap_err();
    assert_eq!(rpc_code(&unprefixed), Some(-32602));

    client.cancel().await.unwrap();
    handle.stop().await;
}

#[tokio::test]
async fn bad_handshakes_are_rejected() {
    let fixture = Fixture::new().await;
    let handle = McpServer::start(fixture.config()).await.unwrap();
    let discovery = read_discovery(&fixture);

    for line in [
        b"{\"bridge\":1,\"token\":\"wrong\"}\n".to_vec(),
        b"this is not json\n".to_vec(),
        b"{\"bridge\":9,\"token\":\"x\"}\n".to_vec(),
        [vec![b'a'; 600], b"\n".to_vec()].concat(),
    ] {
        let (mut stream, reply) = raw_handshake(&discovery.endpoint, &line).await;
        assert_eq!(reply.trim(), "{\"ok\":false}");
        assert_closed(&mut stream).await;
    }

    // A peer that never says anything is cut off after the timeout.
    let mut silent = UnixStream::connect(&discovery.endpoint).await.unwrap();
    let mut buf = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(8), silent.read_to_end(&mut buf))
        .await
        .expect("silent handshake must time out")
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&buf[..read]).trim(),
        "{\"ok\":false}"
    );

    // The server is still healthy.
    let client = connect(&discovery, "after-bad").await;
    assert_eq!(client.peer().list_all_tools().await.unwrap().len(), 22);
    client.cancel().await.unwrap();
    handle.stop().await;
}

#[tokio::test]
async fn oversized_message_ends_the_session() {
    let fixture = Fixture::new().await;
    let handle = McpServer::start(fixture.config()).await.unwrap();
    let discovery = read_discovery(&fixture);
    let hello = format!("{{\"bridge\":1,\"token\":\"{}\"}}\n", discovery.token);
    let (mut stream, reply) = raw_handshake(&discovery.endpoint, hello.as_bytes()).await;
    assert_eq!(reply.trim(), "{\"ok\":true}");

    let junk = vec![b'x'; MAX_MESSAGE_BYTES + 10];
    // The server may close while we are still writing.
    let _ = stream.write_all(&junk).await;
    let mut rest = Vec::new();
    let result = tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut rest))
        .await
        .expect("session must end");
    assert!(result.is_err() || rest.is_empty());

    // Other sessions are unaffected.
    let client = connect(&discovery, "after-oversize").await;
    assert_eq!(client.peer().list_all_tools().await.unwrap().len(), 22);
    client.cancel().await.unwrap();
    handle.stop().await;
}

#[tokio::test]
async fn endpoint_and_discovery_have_private_permissions() {
    let fixture = Fixture::new().await;
    let data_mode_before = std::fs::metadata(fixture.data_dir())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    let handle = McpServer::start(fixture.config()).await.unwrap();
    let discovery = read_discovery(&fixture);

    let mode =
        |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(std::path::Path::new(&discovery.endpoint)), 0o600);
    assert_eq!(mode(&discovery_path(fixture.data_dir())), 0o600);
    let socket_dir = std::path::Path::new(&discovery.endpoint).parent().unwrap();
    assert_eq!(mode(socket_dir), 0o700);
    assert_eq!(socket_dir, fixture.data_dir().join("mcp"));
    assert_eq!(
        mode(fixture.data_dir()),
        data_mode_before,
        "data dir must not be chmod-ed"
    );
    assert!(std::path::Path::new(&discovery.endpoint).is_absolute());
    assert_eq!(discovery.token.len(), 64);
    assert_eq!(discovery.pid, std::process::id());
    handle.stop().await;
}

#[tokio::test]
async fn discovery_removal_only_for_the_matching_instance() {
    let fixture = Fixture::new().await;
    let handle = McpServer::start(fixture.config()).await.unwrap();
    let path = discovery_path(fixture.data_dir());

    // Someone else (a newer instance) replaces the file while we run.
    let mut other = read_discovery(&fixture);
    other.instance_id = "someone-else".into();
    std::fs::write(&path, serde_json::to_vec(&other).unwrap()).unwrap();

    handle.stop().await;
    assert!(path.exists(), "a foreign discovery file must be left alone");

    // Normal case: our own file is removed.
    let handle = McpServer::start(fixture.config()).await.unwrap();
    assert!(path.exists());
    handle.stop().await;
    assert!(!path.exists());
}

#[tokio::test]
async fn token_rotates_and_restart_works() {
    let fixture = Fixture::new().await;
    let mut tokens = Vec::new();
    for _ in 0..3 {
        let handle = McpServer::start(fixture.config()).await.unwrap();
        let discovery = read_discovery(&fixture);
        tokens.push(discovery.token.clone());
        let client = connect(&discovery, "restart").await;
        assert_eq!(client.peer().list_all_tools().await.unwrap().len(), 22);
        client.cancel().await.unwrap();
        handle.stop().await;
    }
    tokens.sort();
    tokens.dedup();
    assert_eq!(tokens.len(), 3);
}

#[tokio::test]
async fn stop_disconnects_sessions_and_refuses_new_connections() {
    let fixture = Fixture::new().await;
    let handle = McpServer::start(fixture.config()).await.unwrap();
    let discovery = read_discovery(&fixture);
    let client = connect(&discovery, "stopper").await;
    assert_eq!(client.peer().list_all_tools().await.unwrap().len(), 22);

    handle.stop().await;
    assert_eq!(handle.status(), McpStatus::Off);

    let after = client.peer().list_all_tools().await;
    assert!(after.is_err(), "session must be closed after stop");
    assert!(UnixStream::connect(&discovery.endpoint).await.is_err());
    assert!(!discovery_path(fixture.data_dir()).exists());
    // Idempotent.
    handle.stop().await;
}

#[tokio::test]
async fn connections_beyond_the_cap_are_refused() {
    let fixture = Fixture::new().await;
    let handle = McpServer::start(fixture.config()).await.unwrap();
    let discovery = read_discovery(&fixture);

    let mut clients = Vec::new();
    for index in 0..MAX_CONNECTIONS {
        clients.push(connect(&discovery, &format!("client-{index}")).await);
    }
    assert!(matches!(
        handle.status(),
        McpStatus::Running { ref clients } if clients.len() == MAX_CONNECTIONS
    ));

    let hello = format!("{{\"bridge\":1,\"token\":\"{}\"}}\n", discovery.token);
    let (mut stream, reply) = raw_handshake(&discovery.endpoint, hello.as_bytes()).await;
    assert_eq!(reply.trim(), "{\"ok\":false}");
    assert_closed(&mut stream).await;

    // Freeing a slot lets a new one in.
    clients.pop().unwrap().cancel().await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let again = connect(&discovery, "again").await;
    again.cancel().await.unwrap();
    for client in clients {
        client.cancel().await.unwrap();
    }
    handle.stop().await;
}

#[tokio::test]
async fn second_server_on_the_same_data_dir_reports_endpoint_in_use() {
    let fixture = Fixture::new().await;
    let first = McpServer::start(fixture.config()).await.unwrap();
    let error = McpServer::start(fixture.config()).await.err().unwrap();
    assert_eq!(error.code, mailx_mcp_server::McpFailureCode::EndpointInUse);
    // The failed start must not have touched the first server's files.
    let client = connect(&read_discovery(&fixture), "still-alive").await;
    assert_eq!(client.peer().list_all_tools().await.unwrap().len(), 22);
    client.cancel().await.unwrap();
    first.stop().await;
}

#[tokio::test]
async fn long_data_dir_falls_back_to_a_short_private_socket_path() {
    let fixture = Fixture::new().await;
    let long = fixture.data_dir().join("a".repeat(110));
    std::fs::create_dir_all(&long).unwrap();
    let mut config = fixture.config();
    config.data_dir = long.clone();
    let handle = McpServer::start(config).await.unwrap();

    let bytes = std::fs::read(discovery_path(&long)).unwrap();
    let discovery: Discovery = serde_json::from_slice(&bytes).unwrap();
    let endpoint = std::path::Path::new(&discovery.endpoint);
    assert!(discovery.endpoint.len() <= mailx_mcp_server::MAX_SOCKET_PATH_BYTES);
    assert!(!endpoint.starts_with(&long));
    assert_eq!(
        std::fs::metadata(endpoint.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let client = connect(&discovery, "long-path").await;
    assert_eq!(client.peer().list_all_tools().await.unwrap().len(), 22);
    client.cancel().await.unwrap();
    handle.stop().await;
    assert!(!endpoint.exists());
}

#[tokio::test]
async fn open_session_is_rejected_per_call_once_disabled() {
    let fixture = Fixture::new().await;
    let handle = McpServer::start(fixture.config()).await.unwrap();
    let client = connect(&read_discovery(&fixture), "per-call").await;
    assert_eq!(client.peer().list_all_tools().await.unwrap().len(), 22);

    // Flip the flag without aborting the session: the call itself must fail.
    handle.disable_calls_only_for_tests();
    let call_error = call(&client, "mailx_list_accounts", json!({}))
        .await
        .unwrap_err();
    assert!(
        matches!(call_error, ServiceError::McpError(_)),
        "{call_error:?}"
    );
    assert!(client.peer().list_all_tools().await.is_err());

    client.cancel().await.unwrap();
    handle.stop().await;
}

/// Meta a stateless (2026-07-28) client puts on every request.
fn stateless_meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": {
            "name": "claude-code", "title": "Claude Code", "version": "2.1.316"
        }
    })
}

async fn rpc(
    reader: &mut BufReader<tokio::io::ReadHalf<UnixStream>>,
    writer: &mut tokio::io::WriteHalf<UnixStream>,
    id: u64,
    method: &str,
    params: Value,
) -> Value {
    let line = format!(
        "{}\n",
        json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
    );
    writer.write_all(line.as_bytes()).await.unwrap();
    let mut reply = String::new();
    tokio::time::timeout(Duration::from_secs(3), reader.read_line(&mut reply))
        .await
        .expect("reply in time")
        .unwrap();
    serde_json::from_str(&reply).unwrap()
}

#[tokio::test]
async fn stateless_client_without_initialize_gets_its_name_from_request_meta() {
    let fixture = Fixture::new().await;
    let handle = McpServer::start(fixture.config()).await.unwrap();
    let discovery = read_discovery(&fixture);
    let hello = format!("{{\"bridge\":1,\"token\":\"{}\"}}\n", discovery.token);
    let (stream, reply) = raw_handshake(&discovery.endpoint, hello.as_bytes()).await;
    assert_eq!(reply.trim(), "{\"ok\":true}");
    let (read, mut write) = tokio::io::split(stream);
    let mut read = BufReader::new(read);

    // server/discover with the full stateless meta succeeds.
    let discover = rpc(
        &mut read,
        &mut write,
        1,
        "server/discover",
        json!({"_meta": stateless_meta()}),
    )
    .await;
    assert!(discover["error"].is_null(), "{discover}");
    assert_eq!(
        discover["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "mailx"
    );
    assert!(
        discover["result"]["supportedVersions"]
            .to_string()
            .contains("2026-07-28")
    );
    assert!(discover["result"]["capabilities"]["tools"].is_object());

    // Without `_meta` rmcp itself rejects the request (-32602) before the
    // handler runs; see docs/contracts/mcp.md.
    let bare = rpc(&mut read, &mut write, 2, "server/discover", json!({})).await;
    assert_eq!(bare["error"]["code"], -32602, "{bare}");

    // No initialize: list and call work on the per-request meta alone.
    let list = rpc(
        &mut read,
        &mut write,
        3,
        "tools/list",
        json!({"_meta": stateless_meta()}),
    )
    .await;
    assert_eq!(
        list["result"]["tools"].as_array().unwrap().len(),
        22,
        "{list}"
    );
    assert!(matches!(
        handle.status(),
        McpStatus::Running { ref clients } if labels(clients) == [Some("Claude Code")]
    ));

    let called = rpc(
        &mut read,
        &mut write,
        6,
        "tools/call",
        json!({
            "name": "mailx_list_accounts",
            "arguments": {},
            "_meta": stateless_meta()
        }),
    )
    .await;
    assert!(called["error"].is_null(), "{called}");
    assert_ne!(called["result"]["isError"], true, "{called}");
    assert!(
        called["result"]["structuredContent"]
            .to_string()
            .contains(&fixture.account_id)
    );

    // Without client info in `_meta` the call still works, just anonymous.
    let anonymous = json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {}
    });
    let called = rpc(
        &mut read,
        &mut write,
        5,
        "tools/call",
        json!({"name": "mailx_list_accounts", "arguments": {}, "_meta": anonymous}),
    )
    .await;
    assert!(called["error"].is_null(), "{called}");

    drop((read, write));
    handle.stop().await;
}

#[tokio::test]
async fn status_lists_the_clients_that_are_connected_now() {
    let fixture = Fixture::new().await;
    let handle = McpServer::start(fixture.config()).await.unwrap();
    let discovery = read_discovery(&fixture);

    let mut titled = Implementation::new("cursor", "1");
    titled.title = Some("Cursor".to_owned());
    let first = connect_as(&discovery, titled).await;
    let second = connect(&discovery, "plain-name").await;
    let wait = |expected: Vec<Option<&'static str>>| {
        let handle = &handle;
        async move {
            for _ in 0..50 {
                if let McpStatus::Running { clients } = handle.status()
                    && labels(&clients) == expected
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            panic!("status never became {expected:?}: {:?}", handle.status());
        }
    };
    wait(vec![Some("Cursor"), Some("plain-name")]).await;

    // A raw session that never identifies itself shows up without a label.
    let hello = format!("{{\"bridge\":1,\"token\":\"{}\"}}\n", discovery.token);
    let (_silent, reply) = raw_handshake(&discovery.endpoint, hello.as_bytes()).await;
    assert_eq!(reply.trim(), "{\"ok\":true}");
    wait(vec![Some("Cursor"), Some("plain-name"), None]).await;

    first.cancel().await.unwrap();
    wait(vec![Some("plain-name"), None]).await;
    second.cancel().await.unwrap();
    handle.stop().await;
}
