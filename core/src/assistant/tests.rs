//! Runs against a scripted streaming model and a real test core.
use std::{
    sync::{Arc, Mutex, atomic::Ordering},
    time::{Duration, Instant},
};

use rusqlite::{Connection, params};
use serde_json::{Value, json};
use tempfile::TempDir;

use super::{
    conversations::{Conversations, MAX_CONVERSATIONS, STOPPED_MARKER, ToolRecord},
    runs::{KEEP_FINISHED_FOR, KEEP_FINISHED_RUNS, MAX_ACTIVE_RUNS, Runs},
    *,
};
use crate::{
    AiModelRef, AiProtocol, CoreEvent, SaveAiConnection,
    ai::client::stream::ChatMessage,
    net::test_support::{MockStream, Recorded, StreamReply, mock_stream},
    secrets::{SecretValue, test_support::MemorySecrets},
    settings::{AssistantMode, AssistantSendMode, SettingsStore},
};

// ------------------------------------------------------------ SSE scripting

fn data(value: Value) -> Vec<u8> {
    format!("data: {value}\n\n").into_bytes()
}

fn named(event: &str, value: Value) -> Vec<u8> {
    format!("event: {event}\ndata: {value}\n\n").into_bytes()
}

fn done() -> Vec<u8> {
    b"data: [DONE]\n\n".to_vec()
}

fn oa_text(text: &str) -> Vec<u8> {
    data(json!({"choices":[{"index":0,"delta":{"content":text}}]}))
}

fn oa_finish(reason: &str) -> Vec<u8> {
    data(json!({"choices":[{"index":0,"delta":{},"finish_reason":reason}]}))
}

fn oa_call(index: u64, id: &str, name: &str, args: &Value) -> Vec<u8> {
    data(json!({"choices":[{"index":0,"delta":{"tool_calls":[{
        "index": index, "id": id, "type": "function",
        "function": {"name": name, "arguments": args.to_string()}
    }]}}]}))
}

/// A reply that only says `text`.
fn say(text: &str) -> Vec<Vec<u8>> {
    vec![oa_text(text), oa_finish("stop"), done()]
}

/// A reply with these tool calls: (id, name, arguments).
fn calls(list: &[(&str, &str, Value)]) -> Vec<Vec<u8>> {
    let mut chunks: Vec<Vec<u8>> = list
        .iter()
        .enumerate()
        .map(|(i, (id, name, args))| oa_call(i as u64, id, name, args))
        .collect();
    chunks.push(oa_finish("tool_calls"));
    chunks.push(done());
    chunks
}

fn an_text_reply(text: &str) -> Vec<Vec<u8>> {
    vec![
        named(
            "message_start",
            json!({"type":"message_start","message":{"id":"m","role":"assistant","content":[],
                   "usage":{"input_tokens":3,"output_tokens":1}}}),
        ),
        named(
            "content_block_start",
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
        ),
        named(
            "content_block_delta",
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":text}}),
        ),
        named(
            "content_block_stop",
            json!({"type":"content_block_stop","index":0}),
        ),
        named(
            "message_delta",
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":4}}),
        ),
        named("message_stop", json!({"type":"message_stop"})),
    ]
}

/// What the mock model answers: request `i` gets reply `i`, the last reply
/// repeats. `hold` keeps the connection open after the last chunk.
#[derive(Default)]
struct Script {
    replies: Vec<Vec<Vec<u8>>>,
    next: usize,
    hold: bool,
    status: Option<u16>,
}

// ----------------------------------------------------------------- fixture

struct Fx {
    _core_dir: TempDir,
    _data_dir: TempDir,
    core: MailxCore,
    db: Connection,
    account: String,
    emails: Vec<String>,
    settings: Arc<SettingsStore>,
    ai: Arc<AiService>,
    events: Arc<Mutex<Vec<CoreEvent>>>,
    connection_id: String,
    script: Arc<Mutex<Script>>,
    server: MockStream,
    assistant: Assistant,
}

impl Fx {
    /// A core with one account and three messages, a model connection to the
    /// mock server and the consent already given.
    async fn new() -> Self {
        Self::build(AiProtocol::OpenaiCompatible, true).await
    }

    async fn build(protocol: AiProtocol, consented: bool) -> Self {
        let core_dir = TempDir::new().unwrap();
        let data_dir = TempDir::new().unwrap();
        let events: Arc<Mutex<Vec<CoreEvent>>> = Arc::default();
        let sink = events.clone();
        let raws: [&[u8]; 3] = [
            b"Content-Type: text/plain\r\n\r\nhello world",
            b"Content-Type: text/plain\r\n\r\nsecond mail",
            b"Content-Type: text/plain\r\n\r\nthird mail",
        ];
        let (core, emails) = MailxCore::body_test_fixture_with_events(
            core_dir.path(),
            tokio::runtime::Handle::current(),
            &raws,
            Arc::new(move |event: &CoreEvent| sink.lock().unwrap().push(event.clone())),
        )
        .unwrap();
        let db = Connection::open(core_dir.path().join("mailx.sqlite3")).unwrap();
        db.pragma_update(None, "foreign_keys", true).unwrap();
        let account: String = db
            .query_row("SELECT id FROM accounts", [], |row| row.get(0))
            .unwrap();

        let script: Arc<Mutex<Script>> = Arc::default();
        let served = script.clone();
        let server = mock_stream(move |_: &Recorded| {
            let mut script = served.lock().unwrap();
            if let Some(status) = script.status {
                return StreamReply::sse_chunks(Vec::new()).status(status);
            }
            if script.replies.is_empty() {
                return StreamReply::sse_chunks(Vec::new());
            }
            let index = script.next.min(script.replies.len() - 1);
            script.next += 1;
            let reply = StreamReply::sse_chunks(script.replies[index].clone());
            if script.hold {
                reply.hold_open()
            } else {
                reply
            }
        })
        .await;

        let settings = Arc::new(SettingsStore::open(data_dir.path()).unwrap());
        let ai = Arc::new(AiService::new(
            settings.clone(),
            Arc::new(MemorySecrets::default()),
            data_dir.path(),
        ));
        let connection_id = ai
            .save_connection(SaveAiConnection {
                id: None,
                provider_id: "custom".into(),
                name: "Mock".into(),
                protocol,
                base_url: format!("{}/v1/", server.http.url()),
                use_proxy: false,
                api_key: Some(SecretValue::new("sk-test".into())),
                verify: false,
            })
            .await
            .unwrap()
            .connection_id;
        ai.set_default_model(Some(model_ref(&connection_id, "m", None)))
            .await
            .unwrap();
        let tools = Tools::new(core.clone(), String::new).with_undo_send_seconds(|| Ok(30));
        let sink = events.clone();
        let assistant = Assistant::new(
            core.clone(),
            ai.clone(),
            settings.clone(),
            tools,
            Arc::new(move |event: &CoreEvent| sink.lock().unwrap().push(event.clone())),
        );
        let fx = Self {
            _core_dir: core_dir,
            _data_dir: data_dir,
            core,
            db,
            account,
            emails,
            settings,
            ai,
            events,
            connection_id,
            script,
            server,
            assistant,
        };
        if consented {
            fx.assistant.grant_consent().await.unwrap();
        }
        fx
    }

    /// Replaces what the model answers from now on.
    fn model(&self, replies: Vec<Vec<Vec<u8>>>) {
        *self.script.lock().unwrap() = Script {
            replies,
            ..Script::default()
        };
    }

    fn hold(&self, replies: Vec<Vec<Vec<u8>>>) {
        *self.script.lock().unwrap() = Script {
            replies,
            hold: true,
            ..Script::default()
        };
    }

    fn configure(&self, change: impl FnOnce(&mut AssistantSettings)) {
        self.settings
            .update_assistant(|assistant| {
                change(assistant);
                Ok(())
            })
            .unwrap();
    }

    fn requests(&self) -> usize {
        self.server.http.count()
    }

    fn request(&self, index: usize) -> Value {
        self.server.http.requests.lock().unwrap()[index].json()
    }

    fn run_events(&self, run_id: &str) -> Vec<AssistantEvent> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                CoreEvent::Assistant(event) if event.run_id == run_id => Some(event.clone()),
                _ => None,
            })
            .collect()
    }

    fn other_account(&self) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.db
            .execute(
                "INSERT INTO accounts(id, revision, display_name, email_address, mailbox_source_id, created_at, updated_at)
                 VALUES (?1, 1, 'Other', 'other@example.test', ?2, '2030-01-01T00:00:00.000Z', '2030-01-01T00:00:00.000Z')",
                params![id, uuid::Uuid::new_v4().to_string()],
            )
            .unwrap();
        id
    }

    fn user_draft(&self) -> crate::Draft {
        self.core
            .create_draft(
                serde_json::from_value(json!({
                    "accountId": self.account, "to": [{"address":"recipient@example.test"}],
                    "cc": [], "bcc": [], "subject": "User draft", "bodyHtml": "<p>Mine</p>",
                    "clientMutationId": uuid::Uuid::new_v4().to_string()
                }))
                .unwrap(),
                "",
            )
            .unwrap()
    }

    async fn chat(&self, message: &str) -> RunSnapshot {
        self.assistant
            .start(chat_request(None, message))
            .await
            .unwrap()
    }

    async fn finished(&self, run_id: &str) -> RunSnapshot {
        finished(&self.assistant, run_id).await
    }

    async fn chat_and_wait(&self, message: &str) -> RunSnapshot {
        let started = self.chat(message).await;
        self.finished(&started.run_id).await
    }
}

fn model_ref(connection_id: &str, model_id: &str, context: Option<u32>) -> AiModelRef {
    AiModelRef {
        connection_id: connection_id.into(),
        model_id: model_id.into(),
        name: model_id.into(),
        context_length: context,
        output_length: None,
        tool_call: None,
    }
}

fn chat_request(conversation_id: Option<&str>, message: &str) -> StartRun {
    StartRun {
        kind: RunKind::Chat {
            conversation_id: conversation_id.map(str::to_owned),
            message: message.into(),
        },
    }
}

fn summary_request() -> StartRun {
    StartRun {
        kind: RunKind::NowSummary {
            since: Some("2000-01-01T00:00:00.000Z".into()),
            day_starts: None,
            fresh: false,
        },
    }
}

fn fresh_summary_request() -> StartRun {
    StartRun {
        kind: RunKind::NowSummary {
            since: Some("2000-01-01T00:00:00.000Z".into()),
            day_starts: None,
            fresh: true,
        },
    }
}

async fn finished(assistant: &Assistant, run_id: &str) -> RunSnapshot {
    for _ in 0..400 {
        let snapshot = assistant.snapshot(run_id).unwrap();
        if snapshot.state.finished() {
            return snapshot;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the run did not finish");
}

async fn until(what: &str, mut condition: impl FnMut() -> bool) {
    for _ in 0..400 {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("timed out waiting for {what}");
}

/// The `role: tool` messages of a recorded OpenAI-style request.
fn tool_messages(request: &Value) -> Vec<String> {
    request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|message| message["role"] == "tool")
        .map(|message| message["content"].as_str().unwrap().to_owned())
        .collect()
}

fn is_read(fx: &Fx, index: usize) -> bool {
    fx.core
        .get_email(&fx.emails[index])
        .unwrap()
        .summary
        .is_read
}

// -------------------------------------------------------------- plain text

#[tokio::test]
async fn a_plain_run_streams_events_in_order_with_increasing_seq() {
    let fx = Fx::new().await;
    fx.model(vec![vec![
        oa_text("Hello"),
        oa_text(" there"),
        oa_finish("stop"),
        done(),
    ]]);
    let started = fx.chat("hi").await;
    assert_eq!(started.kind, RunKindTag::Chat);
    assert!(started.conversation_id.is_some());
    assert_eq!(started.state, RunState::Running);
    let snapshot = fx.finished(&started.run_id).await;
    assert_eq!(snapshot.state, RunState::Completed);
    assert_eq!(snapshot.text, "Hello there");
    assert_eq!(snapshot.finished_reason, Some(FinishedReason::Stop));
    assert_eq!(snapshot.error, None);

    let events = fx.run_events(&started.run_id);
    let seqs: Vec<u64> = events.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, (1..=seqs.len() as u64).collect::<Vec<_>>());
    assert_eq!(snapshot.seq, *seqs.last().unwrap());
    assert!(matches!(
        events.first().unwrap().kind,
        AssistantEventKind::State {
            state: RunState::Running
        }
    ));
    let n = events.len();
    assert!(matches!(
        events[n - 2].kind,
        AssistantEventKind::Finished {
            reason: FinishedReason::Stop
        }
    ));
    assert!(matches!(
        events[n - 1].kind,
        AssistantEventKind::State {
            state: RunState::Completed
        }
    ));
    let streamed: String = events
        .iter()
        .filter_map(|e| match &e.kind {
            AssistantEventKind::Text { delta } => Some(delta.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(streamed, "Hello there");

    let request = fx.request(0);
    assert_eq!(request["model"], "m");
    assert_eq!(request["stream"], true);
    assert_eq!(request["messages"][0]["role"], "system");
    let system = request["messages"][0]["content"].as_str().unwrap();
    assert!(system.contains("mail assistant") && system.contains("untrusted"));
    assert_eq!(request["messages"][1]["content"], "hi");
    let names: Vec<&str> = request["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"search_emails") && names.contains(&"send_email"));
    assert!(
        !names.contains(&"show_email_in_app"),
        "MCP-only tools stay hidden"
    );
}

#[tokio::test]
async fn the_anthropic_protocol_runs_too() {
    let fx = Fx::build(AiProtocol::Anthropic, true).await;
    fx.model(vec![an_text_reply("Bonjour")]);
    let snapshot = fx.chat_and_wait("salut").await;
    assert_eq!(snapshot.state, RunState::Completed);
    assert_eq!(snapshot.text, "Bonjour");
    let request = fx.request(0);
    assert!(
        request["system"]
            .as_str()
            .unwrap()
            .contains("mail assistant")
    );
    assert_eq!(request["messages"][0]["content"], "salut");
}

#[tokio::test]
async fn a_chat_continues_with_its_history() {
    let fx = Fx::new().await;
    fx.model(vec![say("one"), say("two")]);
    let first = fx.chat_and_wait("first").await;
    let id = first.conversation_id.clone().unwrap();
    let second = fx
        .assistant
        .start(chat_request(Some(&id), "second"))
        .await
        .unwrap();
    assert_eq!(second.conversation_id.as_deref(), Some(id.as_str()));
    fx.finished(&second.run_id).await;
    let request = fx.request(1);
    let roles: Vec<&str> = request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    assert_eq!(roles, ["system", "user", "assistant", "user"]);
    assert_eq!(request["messages"][2]["content"], "one");
}

#[tokio::test]
async fn bad_requests_are_rejected_before_anything_runs() {
    let fx = Fx::new().await;
    for message in ["", "   ", &"x".repeat(MAX_MESSAGE_CHARS + 1)] {
        let error = fx
            .assistant
            .start(chat_request(None, message))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Validation);
    }
    let error = fx
        .assistant
        .start(chat_request(Some("missing"), "hi"))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound);
    assert_eq!(
        fx.assistant.snapshot("nope").unwrap_err().code,
        ErrorCode::NotFound
    );
    assert_eq!(
        fx.assistant.cancel("nope").unwrap_err().code,
        ErrorCode::NotFound
    );
    assert_eq!(fx.requests(), 0);
    let bad: Result<StartRun, _> =
        serde_json::from_value(json!({"kind": {"type": "chat", "message": "x", "extra": 1}}));
    assert!(bad.is_err());
}

// ------------------------------------------------------------------- tools

#[tokio::test]
async fn a_tool_call_runs_against_the_core_and_its_result_goes_back_wrapped() {
    let fx = Fx::new().await;
    fx.model(vec![
        calls(&[("c1", "search_emails", json!({"query": "hello"}))]),
        say("Found it."),
    ]);
    let snapshot = fx.chat_and_wait("find hello").await;
    assert_eq!(snapshot.state, RunState::Completed);
    assert_eq!(snapshot.text, "Found it.");
    assert_eq!(snapshot.tools.len(), 1);
    let tool = &snapshot.tools[0];
    assert_eq!(
        (tool.call_id.as_str(), tool.name.as_str()),
        ("c1", "search_emails")
    );
    assert_eq!(tool.status, ToolStatus::Done);
    assert_eq!(tool.summary, ToolActivitySummary::Found { count: 1 });

    let second = fx.request(1);
    let results = tool_messages(&second);
    assert_eq!(results.len(), 1);
    assert!(results[0].starts_with("<tool_result name=\"search_emails\" trust=\"untrusted\">"));
    assert!(results[0].contains(&fx.emails[0]));
    // The assistant message before it carries the call with its arguments.
    let assistant = second["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "assistant")
        .unwrap();
    assert_eq!(
        assistant["tool_calls"][0]["function"]["name"],
        "search_emails"
    );

    let events = fx.run_events(&snapshot.run_id);
    let statuses: Vec<ToolStatus> = events
        .iter()
        .filter_map(|e| match &e.kind {
            AssistantEventKind::Tool { status, .. } => Some(*status),
            _ => None,
        })
        .collect();
    assert_eq!(statuses, [ToolStatus::Running, ToolStatus::Done]);
    let seqs: Vec<u64> = events.iter().map(|e| e.seq).collect();
    assert!(seqs.windows(2).all(|pair| pair[1] == pair[0] + 1));
}

// ------------------------------------------------------------ confirmation

/// Waits until the run asks something.
async fn pending(assistant: &Assistant, run_id: &str) -> PendingConfirmation {
    for _ in 0..400 {
        let snapshot = assistant.snapshot(run_id).unwrap();
        if let Some(pending) = snapshot.pending {
            assert_eq!(snapshot.state, RunState::AwaitingConfirmation);
            return pending;
        }
        assert!(!snapshot.state.finished(), "ended without asking");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the run never asked");
}

fn send_args(draft: &crate::Draft) -> Value {
    json!({"action": "send", "draftId": draft.id, "expectedRevision": draft.revision,
           "clientMutationId": uuid::Uuid::new_v4().to_string()})
}

fn outbox_count(fx: &Fx) -> usize {
    fx.core.list_outbox(None, true).unwrap().len()
}

impl Fx {
    fn subject(&self, index: usize) -> String {
        self.core
            .get_email(&self.emails[index])
            .unwrap()
            .summary
            .subject
    }

    fn draft_to(&self, address: &str) -> crate::Draft {
        self.core
            .create_draft(
                serde_json::from_value(json!({
                    "accountId": self.account, "to": [{"address": address}],
                    "cc": [], "bcc": [], "subject": "Plans", "bodyHtml": "<p>Hello there</p>",
                    "clientMutationId": uuid::Uuid::new_v4().to_string()
                }))
                .unwrap(),
                "",
            )
            .unwrap()
    }
}

#[tokio::test]
async fn organize_in_ask_mode_pauses_and_approval_runs_the_original_call() {
    let fx = Fx::new().await;
    fx.configure(|a| a.organize = AssistantMode::Ask);
    let args = json!({"emailIds": [fx.emails[0]], "isRead": true});
    fx.model(vec![
        calls(&[("c1", "set_email_flags", args)]),
        say("Marked."),
    ]);
    let started = fx.chat("mark it read").await;
    let asked = pending(&fx.assistant, &started.run_id).await;
    assert_eq!(asked.call_id, "c1");
    assert_eq!(asked.request.action, ConfirmAction::Organize);
    assert_eq!(asked.request.tool, "set_email_flags");
    assert_eq!(
        asked.request.details,
        ConfirmationDetails::Organize {
            change: OrganizeChange::Flags {
                is_read: Some(true),
                is_starred: None
            },
            count: 1,
            subjects: vec![fx.subject(0)],
        }
    );
    assert!(!is_read(&fx, 0), "nothing ran while the question is open");
    assert_eq!(fx.requests(), 1, "the model is not asked again meanwhile");

    fx.assistant
        .respond(&started.run_id, &asked.request_id, true)
        .unwrap();
    let snapshot = fx.finished(&started.run_id).await;
    assert_eq!(snapshot.state, RunState::Completed);
    assert_eq!(snapshot.pending, None);
    assert_eq!(snapshot.tools[0].status, ToolStatus::Done);
    assert!(is_read(&fx, 0));

    // The events: running, the pause with its request, running again.
    let events = fx.run_events(&started.run_id);
    let states: Vec<RunState> = events
        .iter()
        .filter_map(|e| match e.kind {
            AssistantEventKind::State { state } => Some(state),
            _ => None,
        })
        .collect();
    assert_eq!(
        states[..3],
        [
            RunState::Running,
            RunState::AwaitingConfirmation,
            RunState::Running
        ]
    );
    assert!(events.iter().any(|e| matches!(
        &e.kind,
        AssistantEventKind::Confirm { request_id, call_id, .. }
            if *request_id == asked.request_id && call_id == "c1"
    )));
}

#[tokio::test]
async fn denying_tells_the_model_and_changes_nothing() {
    let fx = Fx::new().await;
    fx.configure(|a| a.organize = AssistantMode::Ask);
    fx.model(vec![
        calls(&[(
            "c1",
            "set_email_flags",
            json!({"emailIds": [fx.emails[0]], "isRead": true}),
        )]),
        say("I could not."),
    ]);
    let started = fx.chat("mark it read").await;
    let asked = pending(&fx.assistant, &started.run_id).await;
    fx.assistant
        .respond(&started.run_id, &asked.request_id, false)
        .unwrap();
    let snapshot = fx.finished(&started.run_id).await;
    assert_eq!(snapshot.state, RunState::Completed);
    assert_eq!(snapshot.tools[0].status, ToolStatus::Denied);
    assert_eq!(snapshot.tools[0].summary, ToolActivitySummary::Denied);
    assert!(!is_read(&fx, 0));
    let results = tool_messages(&fx.request(1));
    assert!(results[0].contains("declined"), "{results:?}");
    assert!(!results[0].contains("tool_result"), "not tool output");
}

#[tokio::test]
async fn direct_organize_carries_undo_data_for_the_ui_and_never_for_the_model() {
    let fx = Fx::new().await;
    fx.model(vec![
        calls(&[
            (
                "f",
                "set_email_flags",
                json!({"emailIds": [fx.emails[0]], "isRead": true}),
            ),
            ("a", "archive_emails", json!({"emailIds": [fx.emails[1]]})),
        ]),
        say("Done."),
    ]);
    let snapshot = fx.chat_and_wait("tidy up").await;
    let tool = |id: &str| snapshot.tools.iter().find(|t| t.call_id == id).unwrap();
    let ToolActivitySummary::Changed {
        undo: Some(ToolUndo::Flags { restore }),
        ..
    } = &tool("f").summary
    else {
        panic!("{:?}", tool("f"));
    };
    assert_eq!(
        restore,
        &vec![FlagRestore {
            email_id: fx.emails[0].clone(),
            is_read: Some(false),
            is_starred: None,
        }]
    );
    let ToolActivitySummary::Changed {
        undo: Some(ToolUndo::Archive { token }),
        ..
    } = &tool("a").summary
    else {
        panic!("{:?}", tool("a"));
    };
    // The same token undoes the archive through the core.
    fx.core.undo_archive(token).unwrap();

    // Nothing the model can read mentions the token or the previous flags.
    let second = fx.request(1).to_string();
    assert!(!second.contains(token), "the token reached the model");
    assert!(!second.contains("undoToken") && !second.contains("restore"));
    for result in tool_messages(&fx.request(1)) {
        assert!(!result.contains(token));
    }
}

#[tokio::test]
async fn sending_in_confirm_mode_pauses_and_approval_queues_the_mail() {
    let fx = Fx::new().await;
    let draft = fx.draft_to("stranger@example.test");
    fx.core
        .add_attachment(
            crate::AddAttachmentRequest {
                draft_id: draft.id.clone(),
                expected_revision: draft.revision,
                file_name: "report.pdf".into(),
            },
            b"hello",
        )
        .unwrap();
    let draft = fx.core.get_draft(&draft.id).unwrap();
    fx.model(vec![
        calls(&[("c1", "send_email", send_args(&draft))]),
        say("Queued."),
    ]);
    let started = fx.chat("send my draft").await;
    let asked = pending(&fx.assistant, &started.run_id).await;
    assert_eq!(asked.request.action, ConfirmAction::Send);
    assert_eq!(asked.request.tool, "send_email");
    let identity = fx
        .core
        .account_identities()
        .unwrap()
        .into_iter()
        .find(|a| a.id == fx.account)
        .unwrap();
    assert_eq!(
        asked.request.details,
        ConfirmationDetails::Send {
            account_name: identity.display_name,
            account_address: identity.email_address,
            to: vec![crate::MailAddress {
                name: None,
                address: "stranger@example.test".into()
            }],
            cc: vec![],
            bcc: vec![],
            subject: "Plans".into(),
            body_preview: "Hello there".into(),
            attachments: vec![ConfirmAttachment {
                file_name: "report.pdf".into(),
                size: 5
            }],
            warnings: vec![
                ConfirmWarning::NewRecipient,
                ConfirmWarning::NotInConversation,
                ConfirmWarning::HasAttachments
            ],
        }
    );
    assert_eq!(outbox_count(&fx), 0, "nothing queued while asking");

    fx.assistant
        .respond(&started.run_id, &asked.request_id, true)
        .unwrap();
    let snapshot = fx.finished(&started.run_id).await;
    assert_eq!(snapshot.tools[0].status, ToolStatus::Done);
    let ToolActivitySummary::Outbox { outbox_id } = &snapshot.tools[0].summary else {
        panic!("{:?}", snapshot.tools[0].summary);
    };
    let item = fx.core.get_outbox(outbox_id).unwrap();
    assert_eq!(item.draft_id.as_deref(), Some(draft.id.as_str()));
}

#[tokio::test]
async fn denying_a_send_queues_nothing_and_direct_mode_never_asks() {
    let fx = Fx::new().await;
    let draft = fx.user_draft();
    fx.model(vec![
        calls(&[("c1", "send_email", send_args(&draft))]),
        say("Not sent."),
    ]);
    let started = fx.chat("send my draft").await;
    let asked = pending(&fx.assistant, &started.run_id).await;
    fx.assistant
        .respond(&started.run_id, &asked.request_id, false)
        .unwrap();
    let snapshot = fx.finished(&started.run_id).await;
    assert_eq!(snapshot.tools[0].status, ToolStatus::Denied);
    assert_eq!(outbox_count(&fx), 0);
    assert!(tool_messages(&fx.request(1))[0].contains("declined"));

    let fx = Fx::new().await;
    fx.configure(|a| a.send = AssistantSendMode::Direct);
    let draft = fx.user_draft();
    fx.model(vec![
        calls(&[("c1", "send_email", send_args(&draft))]),
        say("Queued."),
    ]);
    let snapshot = fx.chat_and_wait("send my draft").await;
    assert_eq!(snapshot.tools[0].status, ToolStatus::Done);
    assert!(
        fx.run_events(&snapshot.run_id)
            .iter()
            .all(|e| !matches!(e.kind, AssistantEventKind::Confirm { .. }))
    );
    assert_eq!(outbox_count(&fx), 1);
}

#[tokio::test]
async fn send_warnings_come_from_the_cache_and_from_what_this_run_read() {
    let fx = Fx::new().await;
    // The first message is from a known correspondent.
    fx.db
        .execute(
            "UPDATE messages SET from_address = 'friend@example.test' WHERE id = ?1",
            params![fx.emails[0]],
        )
        .unwrap();
    let friend = fx.draft_to("Friend@Example.test");
    // Read the friend's mail first in the same turn: known and in conversation.
    fx.model(vec![
        calls(&[
            ("r", "read_email", json!({"emailId": fx.emails[0]})),
            ("s", "send_email", send_args(&friend)),
        ]),
        say("Done."),
    ]);
    let started = fx.chat("answer my friend").await;
    let asked = pending(&fx.assistant, &started.run_id).await;
    assert_eq!(asked.call_id, "s");
    let ConfirmationDetails::Send { warnings, .. } = &asked.request.details else {
        panic!("{:?}", asked.request);
    };
    assert_eq!(warnings, &Vec::<ConfirmWarning>::new());
    fx.assistant
        .respond(&started.run_id, &asked.request_id, false)
        .unwrap();
    fx.finished(&started.run_id).await;

    // Known in the cache but not read in this run: only notInConversation.
    let again = fx.draft_to("friend@example.test");
    fx.model(vec![
        calls(&[("s", "send_email", send_args(&again))]),
        say("Done."),
    ]);
    let started = fx.chat("answer my friend").await;
    let asked = pending(&fx.assistant, &started.run_id).await;
    let ConfirmationDetails::Send { warnings, .. } = &asked.request.details else {
        panic!("{:?}", asked.request);
    };
    assert_eq!(warnings, &vec![ConfirmWarning::NotInConversation]);
    fx.assistant.cancel(&started.run_id).unwrap();
}

#[tokio::test]
async fn the_request_shows_the_draft_as_it_is_not_as_the_model_describes_it() {
    let fx = Fx::new().await;
    let draft = fx.draft_to("someone@example.test");
    let mut args = send_args(&draft);
    // Model text that disagrees with the record is ignored.
    args["subject"] = json!("Harmless");
    fx.model(vec![calls(&[("c1", "send_email", args)]), say("x")]);
    let started = fx.chat("send").await;
    let asked = pending(&fx.assistant, &started.run_id).await;
    // Extra fields make the tool reject the call, but the card comes first
    // and shows the record; approving then fails closed.
    let ConfirmationDetails::Send { subject, .. } = &asked.request.details else {
        panic!("{:?}", asked.request);
    };
    assert_eq!(subject, "Plans");
    fx.assistant
        .respond(&started.run_id, &asked.request_id, true)
        .unwrap();
    let snapshot = fx.finished(&started.run_id).await;
    assert_eq!(snapshot.tools[0].status, ToolStatus::Failed);
    assert_eq!(outbox_count(&fx), 0);
}

#[tokio::test]
async fn a_draft_changed_after_the_question_makes_the_approved_send_fail() {
    let fx = Fx::new().await;
    let draft = fx.draft_to("someone@example.test");
    fx.model(vec![
        calls(&[("c1", "send_email", send_args(&draft))]),
        say("x"),
    ]);
    let started = fx.chat("send").await;
    let asked = pending(&fx.assistant, &started.run_id).await;
    fx.core
        .update_draft(crate::UpdateDraftRequest {
            id: draft.id.clone(),
            expected_revision: draft.revision,
            account_id: None,
            to: draft.to.clone(),
            cc: vec![],
            bcc: vec![],
            subject: "Changed meanwhile".into(),
            body_html: "<p>Other</p>".into(),
        })
        .unwrap();
    fx.assistant
        .respond(&started.run_id, &asked.request_id, true)
        .unwrap();
    let snapshot = fx.finished(&started.run_id).await;
    assert!(matches!(
        &snapshot.tools[0].summary,
        ToolActivitySummary::Error { code } if code == "conflict"
    ));
    assert_eq!(outbox_count(&fx), 0);
}

#[tokio::test]
async fn retry_and_cancel_of_queued_sends_summarise_the_queue_item() {
    let fx = Fx::new().await;
    fx.configure(|a| a.send = AssistantSendMode::Direct);
    let draft = fx.draft_to("someone@example.test");
    fx.model(vec![
        calls(&[("s", "send_email", send_args(&draft))]),
        say("Queued."),
    ]);
    let snapshot = fx.chat_and_wait("send").await;
    let ToolActivitySummary::Outbox { outbox_id } = &snapshot.tools[0].summary else {
        panic!();
    };
    let outbox_id = outbox_id.clone();
    fx.configure(|a| a.send = AssistantSendMode::Confirm);

    // Two calls in one turn: each pauses in order.
    fx.model(vec![
        calls(&[
            (
                "r",
                "send_email",
                json!({"action": "retry", "outboxId": outbox_id}),
            ),
            ("c", "cancel_outbox", json!({"outboxId": outbox_id})),
        ]),
        say("Done."),
    ]);
    let started = fx.chat("deal with the queue").await;
    let first = pending(&fx.assistant, &started.run_id).await;
    assert_eq!(first.call_id, "r");
    assert_eq!(first.request.action, ConfirmAction::Send);
    let expected = |retry| ConfirmationDetails::Outbox {
        retry,
        sender_address: fx.core.get_outbox(&outbox_id).unwrap().sender_email_address,
        subject: "Plans".into(),
        recipients: vec!["someone@example.test".into()],
        recipient_count: 1,
    };
    assert_eq!(first.request.details, expected(true));
    fx.assistant
        .respond(&started.run_id, &first.request_id, false)
        .unwrap();
    let second = loop {
        let next = pending(&fx.assistant, &started.run_id).await;
        if next.request_id != first.request_id {
            break next;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(second.call_id, "c");
    assert_eq!(second.request.action, ConfirmAction::CancelSend);
    assert_eq!(second.request.details, expected(false));
    fx.assistant
        .respond(&started.run_id, &second.request_id, true)
        .unwrap();
    let snapshot = fx.finished(&started.run_id).await;
    assert_eq!(snapshot.tools[0].status, ToolStatus::Denied);
    assert_eq!(snapshot.tools[1].status, ToolStatus::Done);
    assert_eq!(
        fx.core.get_outbox(&outbox_id).unwrap().state,
        crate::OutboxState::Canceled
    );
}

#[tokio::test]
async fn draft_tools_ask_by_whose_draft_it_is_and_describe_the_change() {
    let fx = Fx::new().await;
    let user = fx.user_draft();
    fx.model(vec![
        calls(&[(
            "u",
            "update_draft",
            json!({"draftId": user.id, "expectedRevision": user.revision,
                   "subject": "New", "to": [{"address": "x@example.test"}]}),
        )]),
        say("Done."),
    ]);
    let started = fx.chat("edit my draft").await;
    let asked = pending(&fx.assistant, &started.run_id).await;
    assert_eq!(asked.request.action, ConfirmAction::UserDraft);
    assert_eq!(
        asked.request.details,
        ConfirmationDetails::Draft {
            subject: "User draft".into(),
            origin: crate::DraftOrigin::User,
            change: ConfirmDraftChange::Update {
                fields: vec!["to".into(), "subject".into()]
            },
        }
    );
    fx.assistant
        .respond(&started.run_id, &asked.request_id, true)
        .unwrap();
    fx.finished(&started.run_id).await;
    assert_eq!(fx.core.get_draft(&user.id).unwrap().subject, "New");

    // Delete and attachment changes on a draft the assistant made, with
    // aiDrafts set to ask.
    fx.configure(|a| a.ai_drafts = AssistantMode::Ask);
    fx.model(vec![
        calls(&[(
            "n",
            "create_draft",
            json!({"accountId": fx.account, "to": [{"address": "a@example.test"}],
                   "subject": "From AI", "bodyHtml": "<p>x</p>"}),
        )]),
        say("Drafted."),
    ]);
    let started = fx.chat("draft something").await;
    let asked = pending(&fx.assistant, &started.run_id).await;
    assert_eq!(asked.request.action, ConfirmAction::AiDraft);
    assert_eq!(
        asked.request.details,
        ConfirmationDetails::Draft {
            subject: "From AI".into(),
            origin: crate::DraftOrigin::Ai,
            change: ConfirmDraftChange::Create,
        }
    );
    fx.assistant
        .respond(&started.run_id, &asked.request_id, true)
        .unwrap();
    let snapshot = fx.finished(&started.run_id).await;
    let ToolActivitySummary::Draft { draft_id, .. } = &snapshot.tools[0].summary else {
        panic!("{:?}", snapshot.tools[0]);
    };
    let ai = fx.core.get_draft(draft_id).unwrap();
    fx.model(vec![
        calls(&[
            (
                "a",
                "add_draft_attachment",
                json!({"draftId": ai.id, "expectedRevision": ai.revision,
                       "filePath": "/tmp/some/notes.txt"}),
            ),
            (
                "d",
                "delete_draft",
                json!({"draftId": ai.id, "expectedRevision": ai.revision}),
            ),
        ]),
        say("Done."),
    ]);
    let started = fx.chat("clean up").await;
    let first = pending(&fx.assistant, &started.run_id).await;
    assert_eq!(first.request.action, ConfirmAction::AiDraft);
    assert_eq!(
        first.request.details,
        ConfirmationDetails::Draft {
            subject: "From AI".into(),
            origin: crate::DraftOrigin::Ai,
            change: ConfirmDraftChange::AttachmentAdded {
                file_name: "notes.txt".into()
            },
        }
    );
    fx.assistant
        .respond(&started.run_id, &first.request_id, false)
        .unwrap();
    let second = loop {
        let next = pending(&fx.assistant, &started.run_id).await;
        if next.request_id != first.request_id {
            break next;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    assert_eq!(
        second.request.details,
        ConfirmationDetails::Draft {
            subject: "From AI".into(),
            origin: crate::DraftOrigin::Ai,
            change: ConfirmDraftChange::Delete,
        }
    );
    fx.assistant.cancel(&started.run_id).unwrap();
}

#[tokio::test]
async fn an_unanswered_question_times_out_as_declined() {
    let fx = Fx::new().await;
    fx.assistant
        .set_confirmation_timeout(Duration::from_millis(150));
    fx.configure(|a| a.organize = AssistantMode::Ask);
    fx.model(vec![
        calls(&[(
            "c1",
            "set_email_flags",
            json!({"emailIds": [fx.emails[0]], "isRead": true}),
        )]),
        say("Not done."),
    ]);
    let started = fx.chat("mark it read").await;
    let asked = pending(&fx.assistant, &started.run_id).await;
    let snapshot = fx.finished(&started.run_id).await;
    assert_eq!(snapshot.state, RunState::Completed);
    assert_eq!(snapshot.tools[0].status, ToolStatus::Denied);
    assert!(!is_read(&fx, 0));
    assert!(tool_messages(&fx.request(1))[0].contains("did not answer"));
    // A late answer finds nothing to answer.
    let late = fx
        .assistant
        .respond(&started.run_id, &asked.request_id, true)
        .unwrap_err();
    assert_eq!(late.code, ErrorCode::Conflict);
    assert!(!is_read(&fx, 0));
}

#[tokio::test]
async fn canceling_while_waiting_ends_the_run_canceled() {
    let fx = Fx::new().await;
    fx.configure(|a| a.organize = AssistantMode::Ask);
    fx.model(vec![
        calls(&[(
            "c1",
            "set_email_flags",
            json!({"emailIds": [fx.emails[0]], "isRead": true}),
        )]),
        say("x"),
    ]);
    let started = fx.chat("mark it read").await;
    let asked = pending(&fx.assistant, &started.run_id).await;
    fx.assistant.cancel(&started.run_id).unwrap();
    let snapshot = fx.assistant.snapshot(&started.run_id).unwrap();
    assert_eq!(snapshot.state, RunState::Canceled);
    assert_eq!(snapshot.pending, None);
    assert_eq!(
        fx.assistant
            .respond(&started.run_id, &asked.request_id, true)
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!is_read(&fx, 0));
    assert_eq!(fx.requests(), 1);
    // The conversation is free again.
    fx.model(vec![say("back")]);
    let id = snapshot.conversation_id.unwrap();
    let next = fx
        .assistant
        .start(chat_request(Some(&id), "go on"))
        .await
        .unwrap();
    assert_eq!(fx.finished(&next.run_id).await.text, "back");
}

#[tokio::test]
async fn only_the_pending_request_of_that_run_can_be_answered_once() {
    let fx = Fx::new().await;
    fx.configure(|a| a.organize = AssistantMode::Ask);
    fx.model(vec![
        calls(&[(
            "c1",
            "set_email_flags",
            json!({"emailIds": [fx.emails[0]], "isRead": true}),
        )]),
        say("ok"),
    ]);
    let started = fx.chat("mark it read").await;
    let asked = pending(&fx.assistant, &started.run_id).await;
    let code = |result: Result<(), AppError>| result.unwrap_err().code;
    assert_eq!(
        code(fx.assistant.respond(&started.run_id, "nope", true)),
        ErrorCode::Conflict
    );
    assert_eq!(
        code(fx.assistant.respond("no-such-run", &asked.request_id, true)),
        ErrorCode::NotFound
    );
    // Another run cannot answer this run's question.
    let other = fx.chat("hello").await;
    assert_eq!(
        code(fx.assistant.respond(&other.run_id, &asked.request_id, true)),
        ErrorCode::Conflict
    );
    assert!(!is_read(&fx, 0), "wrong answers changed nothing");
    assert_eq!(
        fx.assistant.snapshot(&started.run_id).unwrap().state,
        RunState::AwaitingConfirmation
    );
    fx.assistant
        .respond(&started.run_id, &asked.request_id, true)
        .unwrap();
    assert_eq!(
        code(
            fx.assistant
                .respond(&started.run_id, &asked.request_id, true)
        ),
        ErrorCode::Conflict,
        "once"
    );
    fx.finished(&started.run_id).await;
    assert!(is_read(&fx, 0));
    assert_eq!(
        code(
            fx.assistant
                .respond(&started.run_id, &asked.request_id, true)
        ),
        ErrorCode::Conflict,
        "after the run"
    );
}

#[tokio::test]
async fn the_same_call_is_not_asked_again_after_a_denial_in_that_run() {
    let fx = Fx::new().await;
    fx.configure(|a| a.organize = AssistantMode::Ask);
    let args = json!({"emailIds": [fx.emails[0]], "isRead": true});
    fx.model(vec![
        calls(&[("c1", "set_email_flags", args.clone())]),
        calls(&[("c2", "set_email_flags", args)]),
        say("Fine, I stop."),
    ]);
    let started = fx.chat("mark it read").await;
    let asked = pending(&fx.assistant, &started.run_id).await;
    fx.assistant
        .respond(&started.run_id, &asked.request_id, false)
        .unwrap();
    let snapshot = fx.finished(&started.run_id).await;
    assert_eq!(snapshot.state, RunState::Completed);
    assert_eq!(fx.requests(), 3);
    let confirms = fx
        .run_events(&started.run_id)
        .iter()
        .filter(|e| matches!(e.kind, AssistantEventKind::Confirm { .. }))
        .count();
    assert_eq!(confirms, 1, "asked once");
    assert!(
        snapshot
            .tools
            .iter()
            .all(|t| t.status == ToolStatus::Denied)
    );
    let results = tool_messages(&fx.request(2));
    assert!(results[1].contains("already declined"), "{results:?}");
    assert!(!is_read(&fx, 0));
}

#[tokio::test]
async fn an_excluded_account_is_invisible_to_the_tools() {
    let fx = Fx::new().await;
    let other = fx.other_account();
    fx.configure(|a| a.excluded_accounts = vec![fx.account.clone()]);
    fx.model(vec![
        calls(&[
            ("a", "list_accounts", json!({})),
            ("b", "read_email", json!({"emailId": fx.emails[0]})),
            ("c", "search_emails", json!({"query": "hello"})),
        ]),
        say("Only the other account."),
    ]);
    let snapshot = fx.chat_and_wait("what do I have").await;
    assert_eq!(snapshot.state, RunState::Completed);
    let results = tool_messages(&fx.request(1));
    assert!(results[0].contains(&other) && !results[0].contains(&fx.account));
    assert!(results[1].contains("notFound"), "{}", results[1]);
    assert_eq!(
        snapshot
            .tools
            .iter()
            .find(|t| t.call_id == "b")
            .unwrap()
            .status,
        ToolStatus::Failed
    );
    assert_eq!(
        snapshot
            .tools
            .iter()
            .find(|t| t.call_id == "c")
            .unwrap()
            .summary,
        ToolActivitySummary::Found { count: 0 }
    );
    let body = fx.request(1).to_string();
    assert!(
        !body.contains("hello world"),
        "the mail text never reached the model"
    );
}

#[tokio::test]
async fn tool_errors_reach_the_model_as_error_results_with_the_stable_code() {
    let fx = Fx::new().await;
    fx.model(vec![
        calls(&[
            (
                "a",
                "read_email",
                json!({"emailId": uuid::Uuid::new_v4().to_string()}),
            ),
            ("b", "no_such_tool", json!({})),
        ]),
        say("Sorry."),
    ]);
    let snapshot = fx.chat_and_wait("read nope").await;
    assert_eq!(snapshot.state, RunState::Completed);
    let a = &snapshot.tools[0];
    assert_eq!(a.status, ToolStatus::Failed);
    assert!(matches!(&a.summary, ToolActivitySummary::Error { code } if code == "notFound"));
    // An unknown tool is never run and never asked about.
    assert_eq!(snapshot.tools[1].status, ToolStatus::Denied);
}

#[tokio::test]
async fn every_tool_the_agent_can_see_is_classified() {
    let fx = Fx::new().await;
    let direct = AssistantSettings {
        send: AssistantSendMode::Direct,
        ..AssistantSettings::default()
    };
    for tool in fx
        .assistant
        .inner
        .tools
        .list(crate::tools::Audience::BuiltinAgent)
    {
        assert_eq!(
            policy::classify(tool.id, &direct, policy::DraftSource::Assistant),
            policy::Decision::Run,
            "{} is not in any category",
            tool.id
        );
    }
}

// ------------------------------------------------------------------ limits

#[tokio::test]
async fn the_turn_limit_stops_a_model_that_keeps_calling_tools() {
    let fx = Fx::new().await;
    fx.model(vec![calls(&[("c", "list_accounts", json!({}))])]);
    let snapshot = fx.chat_and_wait("loop forever").await;
    assert_eq!(snapshot.state, RunState::Completed);
    assert_eq!(snapshot.finished_reason, Some(FinishedReason::Limit));
    assert_eq!(fx.requests(), agent::MAX_TURNS);
    // The last turn's call was not run.
    assert_eq!(snapshot.tools.len(), 1, "same call id throughout");
    assert_eq!(snapshot.tools[0].status, ToolStatus::Failed);
    // The conversation is usable again and ends with the assistant.
    fx.model(vec![say("back")]);
    let id = snapshot.conversation_id.unwrap();
    let next = fx
        .assistant
        .start(chat_request(Some(&id), "go on"))
        .await
        .unwrap();
    assert_eq!(fx.finished(&next.run_id).await.text, "back");
}

#[tokio::test]
async fn only_eight_tool_calls_run_per_turn() {
    let fx = Fx::new().await;
    let many: Vec<(String, &str, Value)> = (0..9)
        .map(|i| (format!("c{i}"), "list_accounts", json!({})))
        .collect();
    let borrowed: Vec<(&str, &str, Value)> = many
        .iter()
        .map(|(id, name, args)| (id.as_str(), *name, args.clone()))
        .collect();
    fx.model(vec![calls(&borrowed), say("ok")]);
    let snapshot = fx.chat_and_wait("many").await;
    assert_eq!(snapshot.tools.len(), 9);
    assert!(
        snapshot.tools[..8]
            .iter()
            .all(|t| t.status == ToolStatus::Done)
    );
    assert_eq!(snapshot.tools[8].status, ToolStatus::Failed);
    assert!(
        matches!(&snapshot.tools[8].summary, ToolActivitySummary::Error { code } if code == "limit")
    );
    // Every call still got a result, so the history stays valid.
    assert_eq!(tool_messages(&fx.request(1)).len(), 9);
}

#[tokio::test]
async fn a_conversation_that_outgrows_the_context_stops_with_a_fixed_error() {
    let fx = Fx::new().await;
    fx.ai
        .set_default_model(Some(model_ref(&fx.connection_id, "m", Some(100))))
        .await
        .unwrap();
    let snapshot = fx.chat_and_wait("hi").await;
    assert_eq!(snapshot.state, RunState::Failed);
    assert_eq!(snapshot.error, Some(ErrorCode::AssistantContextFull));
    assert_eq!(fx.requests(), 0, "nothing was sent");
}

#[tokio::test]
async fn at_most_two_runs_are_active_and_a_conversation_takes_one() {
    let fx = Fx::new().await;
    fx.hold(vec![vec![oa_text("...")]]);
    let one = fx.chat("one").await;
    let two = fx.chat("two").await;
    let busy = fx
        .assistant
        .start(chat_request(None, "three"))
        .await
        .unwrap_err();
    assert_eq!(busy.code, ErrorCode::AssistantBusy);
    let same = fx
        .assistant
        .start(chat_request(one.conversation_id.as_deref(), "again"))
        .await
        .unwrap_err();
    assert_eq!(same.code, ErrorCode::AssistantBusy);
    // The refused message did not stay in the conversation.
    assert_eq!(
        fx.assistant
            .inner
            .conversations
            .history(one.conversation_id.as_deref().unwrap())
            .unwrap()
            .len(),
        1
    );
    fx.assistant.cancel(&one.run_id).unwrap();
    let three = fx.chat("three").await;
    assert_eq!(three.state, RunState::Running);
    fx.assistant.cancel(&two.run_id).unwrap();
    fx.assistant.cancel(&three.run_id).unwrap();
    assert_eq!(MAX_ACTIVE_RUNS, 2);
}

// ----------------------------------------------------------------- cancel

#[tokio::test]
async fn cancel_mid_stream_closes_the_connection_and_ends_canceled() {
    let fx = Fx::new().await;
    fx.hold(vec![vec![oa_text("par")]]);
    let started = fx.chat("talk").await;
    let id = started.run_id.clone();
    until("the first text", || {
        fx.assistant.snapshot(&id).unwrap().text == "par"
    })
    .await;
    fx.assistant.cancel(&id).unwrap();
    let snapshot = fx.assistant.snapshot(&id).unwrap();
    assert_eq!(snapshot.state, RunState::Canceled);
    assert_eq!(snapshot.text, "par", "what was said stays");
    until("the connection to close", || {
        fx.server.closed.load(Ordering::SeqCst)
    })
    .await;
    let events = fx.run_events(&id);
    assert!(matches!(
        events.last().unwrap().kind,
        AssistantEventKind::State {
            state: RunState::Canceled
        }
    ));
    // Canceling again, or after the end, is harmless and changes nothing.
    fx.assistant.cancel(&id).unwrap();
    assert_eq!(fx.run_events(&id).len(), events.len());
    // The conversation is free and its history ends with the assistant.
    let conversation = started.conversation_id.unwrap();
    until("the conversation to be freed", || {
        fx.assistant.inner.conversations.is_idle(&conversation)
            && fx
                .assistant
                .inner
                .conversations
                .history(&conversation)
                .is_some_and(|h| matches!(h.last(), Some(ChatMessage::Assistant { .. })))
    })
    .await;
    fx.model(vec![say("fine")]);
    let next = fx
        .assistant
        .start(chat_request(Some(&conversation), "again"))
        .await
        .unwrap();
    assert_eq!(fx.finished(&next.run_id).await.state, RunState::Completed);
}

#[tokio::test]
async fn a_provider_failure_ends_failed_with_only_a_fixed_code() {
    let fx = Fx::new().await;
    fx.script.lock().unwrap().status = Some(401);
    let snapshot = fx.chat_and_wait("hi").await;
    assert_eq!(snapshot.state, RunState::Failed);
    assert_eq!(snapshot.error, Some(ErrorCode::AiKeyRejected));
    let events = fx.run_events(&snapshot.run_id);
    assert!(events.iter().any(|e| matches!(
        e.kind,
        AssistantEventKind::Failed {
            code: ErrorCode::AiKeyRejected
        }
    )));
}

// ---------------------------------------------------------------- consent

#[tokio::test]
async fn without_consent_a_run_awaits_it_and_sends_nothing() {
    let fx = Fx::build(AiProtocol::OpenaiCompatible, false).await;
    fx.model(vec![say("hi")]);
    let snapshot = fx.chat("hello").await;
    assert_eq!(snapshot.state, RunState::AwaitingConsent);
    assert_eq!(snapshot.text, "");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(fx.requests(), 0, "no network request");
    // It is over: it does not count as active and is not running.
    assert_eq!(
        fx.assistant.snapshot(&snapshot.run_id).unwrap().state,
        RunState::AwaitingConsent
    );
    assert_eq!(
        fx.assistant.inner.conversations.len(),
        0,
        "the message was not stored"
    );
    let summary = fx.assistant.start(summary_request()).await.unwrap();
    assert_eq!(summary.state, RunState::AwaitingConsent);
    assert_eq!(fx.requests(), 0);

    let binding = fx.assistant.grant_consent().await.unwrap();
    assert_eq!(binding.connection_id, fx.connection_id);
    assert!(binding.base_url.ends_with("/v1"));
    let started = fx.chat("hello").await;
    assert_eq!(started.state, RunState::Running);
    assert_eq!(
        fx.finished(&started.run_id).await.state,
        RunState::Completed
    );
    assert_eq!(fx.requests(), 1);
}

#[tokio::test]
async fn changing_the_connection_or_its_address_invalidates_the_consent_but_the_model_does_not() {
    let fx = Fx::new().await;
    fx.model(vec![say("ok")]);
    // Another model of the same connection: still consented.
    fx.ai
        .set_default_model(Some(model_ref(&fx.connection_id, "other-model", None)))
        .await
        .unwrap();
    assert_eq!(fx.chat_and_wait("a").await.state, RunState::Completed);
    assert_eq!(fx.request(0)["model"], "other-model");

    // The same connection at another address: consent is void.
    fx.ai
        .save_connection(SaveAiConnection {
            id: Some(fx.connection_id.clone()),
            provider_id: "custom".into(),
            name: "Mock".into(),
            protocol: AiProtocol::OpenaiCompatible,
            base_url: format!("{}/v2/", fx.server.http.url()),
            use_proxy: false,
            api_key: None,
            verify: false,
        })
        .await
        .unwrap();
    let before = fx.requests();
    assert_eq!(fx.chat("b").await.state, RunState::AwaitingConsent);
    assert_eq!(fx.requests(), before);

    // Granting again binds the new address.
    fx.assistant.grant_consent().await.unwrap();
    assert_eq!(fx.chat_and_wait("c").await.state, RunState::Completed);

    // A different connection: consent is void as well.
    let second = fx
        .ai
        .save_connection(SaveAiConnection {
            id: None,
            provider_id: "custom".into(),
            name: "Second".into(),
            protocol: AiProtocol::OpenaiCompatible,
            base_url: format!("{}/v1/", fx.server.http.url()),
            use_proxy: false,
            api_key: None,
            verify: false,
        })
        .await
        .unwrap()
        .connection_id;
    fx.ai
        .set_default_model(Some(model_ref(&second, "m", None)))
        .await
        .unwrap();
    assert_eq!(fx.chat("d").await.state, RunState::AwaitingConsent);
}

#[tokio::test]
async fn revoking_consent_stops_running_runs_and_blocks_new_ones() {
    let fx = Fx::new().await;
    fx.hold(vec![vec![oa_text("x")]]);
    let started = fx.chat("go").await;
    let id = started.run_id.clone();
    until("the first text", || {
        !fx.assistant.snapshot(&id).unwrap().text.is_empty()
    })
    .await;
    fx.assistant.revoke_consent().unwrap();
    assert_eq!(
        fx.assistant.snapshot(&id).unwrap().state,
        RunState::Canceled
    );
    assert_eq!(
        fx.settings.get().unwrap().settings.assistant.consented,
        None
    );
    assert_eq!(fx.chat("again").await.state, RunState::AwaitingConsent);
}

#[tokio::test]
async fn a_disabled_assistant_or_a_missing_model_is_a_fixed_error() {
    let fx = Fx::new().await;
    fx.configure(|a| a.enabled = false);
    let error = fx
        .assistant
        .start(chat_request(None, "hi"))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::AssistantDisabled);
    let error = fx.assistant.start(summary_request()).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::AssistantDisabled);
    assert_eq!(fx.requests(), 0);

    let fx = Fx::new().await;
    fx.ai.set_default_model(None).await.unwrap();
    let error = fx
        .assistant
        .start(chat_request(None, "hi"))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::AiNotConfigured);
    assert_eq!(
        fx.assistant.grant_consent().await.unwrap_err().code,
        ErrorCode::AiNotConfigured
    );
}

#[tokio::test]
async fn a_model_without_tool_calling_chats_without_tools() {
    for (tool_call, expect_tools) in [(Some(false), false), (None, true), (Some(true), true)] {
        let fx = Fx::new().await;
        fx.model(vec![say("ok")]);
        let mut own = model_ref(&fx.connection_id, "m", None);
        own.tool_call = tool_call;
        fx.configure(|a| a.model = Some(own));
        let started = fx.chat("hi").await;
        assert_eq!(started.tools_available, expect_tools, "{tool_call:?}");
        let done = fx.finished(&started.run_id).await;
        assert_eq!(done.state, RunState::Completed);
        assert_eq!(done.tools_available, expect_tools);
        let request = fx.request(0);
        assert_eq!(request.get("tools").is_some(), expect_tools, "{request}");
        let system = request["messages"][0]["content"].as_str().unwrap();
        assert_eq!(system.contains("You have no tools"), !expect_tools);
    }
}

#[tokio::test]
async fn the_assistant_model_overrides_the_default_and_a_dangling_one_falls_back() {
    let fx = Fx::new().await;
    fx.model(vec![say("ok")]);
    let own = model_ref(&fx.connection_id, "assistant-model", None);
    fx.configure(|a| a.model = Some(own));
    fx.chat_and_wait("a").await;
    assert_eq!(fx.request(0)["model"], "assistant-model");
    let ghost = model_ref(&uuid::Uuid::new_v4().to_string(), "ghost", None);
    fx.configure(|a| a.model = Some(ghost));
    fx.chat_and_wait("b").await;
    assert_eq!(fx.request(1)["model"], "m");
}

// ------------------------------------------------------------- now summary

#[tokio::test]
async fn a_now_summary_uses_overview_facts_no_tools_and_is_cached() {
    let fx = Fx::new().await;
    fx.model(vec![say("  You have mail.  ")]);
    let started = fx.assistant.start(summary_request()).await.unwrap();
    assert_eq!(started.kind, RunKindTag::NowSummary);
    assert_eq!(started.conversation_id, None);
    let snapshot = fx.finished(&started.run_id).await;
    assert_eq!(snapshot.state, RunState::Completed);
    assert!(snapshot.tools.is_empty());
    assert!(!snapshot.tools_available);
    let generated = snapshot.completed_at.clone().expect("completed_at is set");
    assert!(
        chrono::DateTime::parse_from_rfc3339(&generated).is_ok() && generated.ends_with('Z'),
        "{generated}"
    );

    let request = fx.request(0);
    assert!(request.get("tools").is_none(), "no tools: {request}");
    assert_eq!(request["max_tokens"], 200);
    let system = request["messages"][0]["content"].as_str().unwrap();
    assert!(system.contains("never change a number") && system.contains("untrusted"));
    let facts = request["messages"][1]["content"].as_str().unwrap();
    assert!(
        facts.starts_with("<facts>") && facts.ends_with("</facts>"),
        "{facts}"
    );
    assert!(
        facts.contains("New mail since the user last looked: 3"),
        "{facts}"
    );
    assert!(facts.contains("Starred messages still unread: 0"));
    assert_eq!(request["messages"].as_array().unwrap().len(), 2);

    // Asked again with the same facts: answered from the cache, instantly.
    let again = fx.assistant.start(summary_request()).await.unwrap();
    assert_eq!(again.state, RunState::Completed);
    assert_eq!(again.text, "You have mail.");
    assert_eq!(again.seq, 0);
    assert_eq!(fx.requests(), 1, "no second model call");
    // The cached answer shows when it was generated, not when it was fetched. The cache is
    // stamped just before the run itself completes, so the two may differ by a few ms either way.
    let cached_at = again.completed_at.clone().expect("cached completed_at");
    assert!(
        chrono::DateTime::parse_from_rfc3339(&cached_at)
            .unwrap()
            .signed_duration_since(chrono::DateTime::parse_from_rfc3339(&generated).unwrap())
            .num_milliseconds()
            .abs()
            < 5_000
    );
    tokio::time::sleep(Duration::from_millis(30)).await;
    let later = fx.assistant.start(summary_request()).await.unwrap();
    assert_eq!(later.completed_at, again.completed_at);
    assert_eq!(
        fx.assistant.snapshot(&again.run_id).unwrap().text,
        "You have mail."
    );

    // Other facts, another model or another prompt version miss the cache.
    fx.model(vec![say("Different.")]);
    fx.ai
        .set_default_model(Some(model_ref(&fx.connection_id, "m2", None)))
        .await
        .unwrap();
    let third = fx.assistant.start(summary_request()).await.unwrap();
    assert_eq!(third.state, RunState::Running);
    assert_eq!(fx.finished(&third.run_id).await.text, "Different.");
    assert_eq!(fx.requests(), 2);
}

#[tokio::test]
async fn a_fresh_now_summary_skips_the_cache_and_replaces_it() {
    let fx = Fx::new().await;
    fx.model(vec![say("First."), say("Second.")]);
    let first = fx.assistant.start(summary_request()).await.unwrap();
    assert_eq!(fx.finished(&first.run_id).await.text, "First.");
    // The same facts: a normal start is answered from the cache.
    let cached = fx.assistant.start(summary_request()).await.unwrap();
    assert_eq!(cached.text, "First.");
    assert_eq!(fx.requests(), 1);
    // A fresh start calls the model again.
    let fresh = fx.assistant.start(fresh_summary_request()).await.unwrap();
    assert_eq!(fresh.state, RunState::Running);
    assert_eq!(fx.finished(&fresh.run_id).await.text, "Second.");
    assert_eq!(fx.requests(), 2);
    // The new answer replaced the old one in the cache.
    let after = fx.assistant.start(summary_request()).await.unwrap();
    assert_eq!(after.state, RunState::Completed);
    assert_eq!(after.text, "Second.");
    assert_eq!(fx.requests(), 2);
}

#[tokio::test]
async fn a_now_summary_leaves_out_what_an_excluded_account_would_reveal() {
    let fx = Fx::new().await;
    fx.configure(|a| a.excluded_accounts = vec![fx.account.clone()]);
    fx.model(vec![say("Quiet.")]);
    let started = fx.assistant.start(summary_request()).await.unwrap();
    fx.finished(&started.run_id).await;
    let facts = fx.request(0)["messages"][1]["content"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(
        facts.contains("New mail since the user last looked: 0"),
        "{facts}"
    );
    assert!(
        !facts.contains("Starred") && !facts.contains("from:"),
        "{facts}"
    );
}

#[tokio::test]
async fn a_failed_summary_is_not_cached() {
    let fx = Fx::new().await;
    fx.script.lock().unwrap().status = Some(500);
    let started = fx.assistant.start(summary_request()).await.unwrap();
    let snapshot = fx.finished(&started.run_id).await;
    assert_eq!(snapshot.error, Some(ErrorCode::AiUnavailable));
    assert!(
        snapshot.completed_at.is_some(),
        "failed runs are stamped too"
    );
    fx.model(vec![say("Now it works.")]);
    let again = fx.assistant.start(summary_request()).await.unwrap();
    assert_eq!(again.state, RunState::Running);
    assert_eq!(fx.finished(&again.run_id).await.text, "Now it works.");
}

#[tokio::test]
async fn a_summary_with_a_bad_query_is_a_validation_error() {
    let fx = Fx::new().await;
    let error = fx
        .assistant
        .start(StartRun {
            kind: RunKind::NowSummary {
                since: Some("yesterday".into()),
                day_starts: None,
                fresh: false,
            },
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Validation);
    assert_eq!(fx.requests(), 0);
}

// -------------------------------------------------------------- bookkeeping

fn quiet_runs() -> Runs {
    Runs::new(Arc::new(|_: &CoreEvent| {}))
}

#[test]
fn finished_runs_are_kept_for_ten_minutes_and_at_most_twenty() {
    let runs = quiet_runs();
    let first = runs
        .create(RunKindTag::Chat, None, RunState::Running)
        .unwrap();
    runs.complete(&first, FinishedReason::Stop);
    assert_eq!(runs.len(), 1);
    runs.sweep_for_test(Instant::now() + KEEP_FINISHED_FOR + Duration::from_secs(1));
    assert_eq!(runs.len(), 0);
    assert_eq!(runs.snapshot(&first).unwrap_err().code, ErrorCode::NotFound);

    let ids: Vec<String> = (0..KEEP_FINISHED_RUNS + 5)
        .map(|_| {
            let id = runs
                .create(RunKindTag::Chat, None, RunState::Running)
                .unwrap();
            runs.complete(&id, FinishedReason::Stop);
            id
        })
        .collect();
    assert_eq!(runs.len(), KEEP_FINISHED_RUNS);
    assert!(runs.snapshot(&ids[0]).is_err(), "the oldest went first");
    assert!(runs.snapshot(ids.last().unwrap()).is_ok());
    // An active run is never swept.
    let active = runs
        .create(RunKindTag::Chat, None, RunState::Running)
        .unwrap();
    runs.sweep_for_test(Instant::now() + KEEP_FINISHED_FOR * 3);
    assert_eq!(runs.snapshot(&active).unwrap().state, RunState::Running);
}

#[test]
fn only_the_first_ending_counts() {
    let runs = quiet_runs();
    let id = runs
        .create(RunKindTag::Chat, None, RunState::Running)
        .unwrap();
    runs.cancel(&id).unwrap();
    // A task that lost the race cannot change what was reported.
    assert!(!runs.text(&id, "late".into()));
    runs.complete(&id, FinishedReason::Stop);
    runs.fail(&id, ErrorCode::Internal);
    let snapshot = runs.snapshot(&id).unwrap();
    assert_eq!(snapshot.state, RunState::Canceled);
    assert_eq!(snapshot.text, "");
    assert_eq!(snapshot.error, None);
}

#[test]
fn conversations_keep_twenty_and_drop_the_least_recently_used() {
    let store = Arc::new(Conversations::default());
    let mut ids = Vec::new();
    for i in 0..MAX_CONVERSATIONS {
        let (guard, history) = store.begin(None, format!("m{i}")).unwrap();
        assert_eq!(history.len(), 1);
        ids.push(guard.id.clone());
        drop(guard);
    }
    // Touch the first so the second is now the oldest.
    drop(store.begin(Some(&ids[0]), "again".into()).unwrap());
    let (_new, _) = store.begin(None, "new".into()).unwrap();
    assert_eq!(store.len(), MAX_CONVERSATIONS);
    assert!(store.history(&ids[0]).is_some());
    assert!(store.history(&ids[1]).is_none());
}

#[test]
fn a_dropped_guard_leaves_history_ending_with_the_assistant() {
    let store = Arc::new(Conversations::default());
    let (guard, _) = store.begin(None, "q".into()).unwrap();
    let id = guard.id.clone();
    drop(guard);
    let history = store.history(&id).unwrap();
    assert_eq!(history.len(), 2);
    assert!(matches!(&history[1], ChatMessage::Assistant { text, .. } if text == STOPPED_MARKER));
    let (guard, _) = store.begin(Some(&id), "q2".into()).unwrap();
    guard.rollback();
    assert_eq!(store.history(&id).unwrap().len(), 2);
}

// ------------------------------------------------------ saved conversations

impl Fx {
    /// A second assistant over the same storage, as after a restart: it has
    /// no conversations in memory.
    fn reopened(&self) -> Assistant {
        let tools = Tools::new(self.core.clone(), String::new).with_undo_send_seconds(|| Ok(30));
        Assistant::new(
            self.core.clone(),
            self.ai.clone(),
            self.settings.clone(),
            tools,
            Arc::new(|_: &CoreEvent| {}),
        )
    }

    async fn list(&self) -> Vec<ConversationSummary> {
        self.assistant
            .list_conversations(Some(MAX_CONVERSATION_PAGE), None)
            .await
            .unwrap()
            .items
    }

    fn stored_contents(&self, id: &str) -> Vec<String> {
        let mut statement = self
            .db
            .prepare(
                "SELECT content FROM assistant_messages WHERE conversation_id = ?1 ORDER BY seq",
            )
            .unwrap();
        statement
            .query_map([id], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    fn stored_rows(&self) -> i64 {
        self.db
            .query_row("SELECT count(*) FROM assistant_messages", [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    /// Waits until the conversation has no run (its last save is done).
    async fn idle(&self, id: &str) {
        until("the conversation to be free", || {
            self.assistant.inner.conversations.is_idle(id)
        })
        .await;
    }

    async fn turns(&self, id: &str) -> Vec<ChatTurn> {
        self.assistant.get_conversation(id).await.unwrap().turns
    }
}

fn assistant_turn(turn: &ChatTurn) -> (&str, &[ChatTurnTool], &[ChatTurnConfirmation]) {
    match turn {
        ChatTurn::Assistant {
            text,
            tools,
            confirmations,
            ..
        } => (text, tools, confirmations),
        ChatTurn::User { .. } => panic!("expected an assistant turn: {turn:?}"),
    }
}

#[tokio::test]
async fn a_conversation_is_saved_and_reads_back_with_tools_and_the_answer_to_a_question() {
    let fx = Fx::new().await;
    fx.configure(|a| a.organize = AssistantMode::Ask);
    let mut first = vec![oa_text("Let me do that. ")];
    first.extend(calls(&[(
        "c1",
        "set_email_flags",
        json!({"emailIds": [fx.emails[0]], "isRead": true}),
    )]));
    fx.model(vec![
        first,
        say("Marked."),
        calls(&[("s1", "search_emails", json!({"query": "hello"}))]),
        say("Found it."),
    ]);
    let started = fx.chat("  Please\n mark   the first mail read  ").await;
    let asked = pending(&fx.assistant, &started.run_id).await;
    fx.assistant
        .respond(&started.run_id, &asked.request_id, true)
        .unwrap();
    fx.finished(&started.run_id).await;
    let id = started.conversation_id.unwrap();
    // The run is reported finished only after it was saved.
    let listed = fx.list().await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, id);
    assert_eq!(listed[0].title, "Please mark the first mail read");
    assert_eq!(listed[0].message_count, 1);

    let second = fx
        .assistant
        .start(chat_request(Some(&id), "now find hello"))
        .await
        .unwrap();
    fx.finished(&second.run_id).await;
    let detail = fx.assistant.get_conversation(&id).await.unwrap();
    assert_eq!(detail.id, id);
    assert_eq!(detail.title, "Please mark the first mail read");
    assert_eq!(detail.turns.len(), 4);
    assert!(matches!(
        &detail.turns[0],
        ChatTurn::User { text, .. } if text == "Please\n mark   the first mail read"
    ));

    let (text, tools, confirmations) = assistant_turn(&detail.turns[1]);
    assert_eq!(text, "Let me do that. Marked.");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].call_id, "c1");
    assert_eq!(tools[0].status, ToolStatus::Done);
    assert_eq!(
        tools[0].text_offset, 16,
        "after the words said before the call"
    );
    // The change could be taken back live; the saved copy cannot be.
    assert!(matches!(
        &tools[0].summary,
        ToolActivitySummary::Changed {
            undo: None,
            count: 1,
            ..
        }
    ));
    assert_eq!(
        confirmations,
        [ChatTurnConfirmation {
            call_id: "c1".into(),
            tool: "set_email_flags".into(),
            action: ConfirmAction::Organize,
            outcome: ConfirmOutcome::Approved,
            text_offset: 16,
        }]
    );
    assert!(matches!(
        &detail.turns[1],
        ChatTurn::Assistant {
            state: TurnState::Completed,
            finished_reason: Some(FinishedReason::Stop),
            error: None,
            ..
        }
    ));
    let (text, tools, confirmations) = assistant_turn(&detail.turns[3]);
    assert_eq!(text, "Found it.");
    assert_eq!(tools[0].summary, ToolActivitySummary::Found { count: 1 });
    assert!(confirmations.is_empty());

    // Nothing that undoes a change or waits for an answer is on disk.
    let stored = fx.stored_contents(&id).join("\n");
    for needle in [
        "\"undo\":{",
        "restore",
        "token",
        "\"pending\":",
        "requestId",
        "bodyPreview",
    ] {
        assert!(!stored.contains(needle), "{needle} was stored");
    }
    // The conversation moved up the list with the second message.
    assert_eq!(fx.list().await[0].message_count, 2);
}

#[tokio::test]
async fn a_denied_or_unanswered_question_is_saved_as_its_outcome() {
    let fx = Fx::new().await;
    fx.configure(|a| a.organize = AssistantMode::Ask);
    fx.assistant
        .set_confirmation_timeout(Duration::from_millis(400));
    fx.model(vec![
        calls(&[("a", "archive_emails", json!({"emailIds": [fx.emails[0]]}))]),
        calls(&[("b", "archive_emails", json!({"emailIds": [fx.emails[1]]}))]),
        say("Done."),
    ]);
    let started = fx.chat("archive two").await;
    let asked = pending(&fx.assistant, &started.run_id).await;
    fx.assistant
        .respond(&started.run_id, &asked.request_id, false)
        .unwrap();
    fx.finished(&started.run_id).await;
    let turns = fx.turns(&started.conversation_id.unwrap()).await;
    let (_, tools, confirmations) = assistant_turn(&turns[1]);
    assert_eq!(tools.len(), 2);
    assert!(tools.iter().all(|t| t.status == ToolStatus::Denied));
    let outcomes: Vec<_> = confirmations
        .iter()
        .map(|c| (c.call_id.as_str(), c.outcome))
        .collect();
    assert_eq!(
        outcomes,
        [
            ("a", ConfirmOutcome::Denied),
            ("b", ConfirmOutcome::Expired)
        ]
    );
}

#[tokio::test]
async fn a_reopened_assistant_continues_a_saved_conversation_with_its_stored_history() {
    let fx = Fx::new().await;
    fx.model(vec![
        calls(&[("s1", "search_emails", json!({"query": "hello"}))]),
        say("One mail."),
        say("Only if you ask."),
    ]);
    let first = fx.chat_and_wait("what is in my box?").await;
    let id = first.conversation_id.unwrap();

    let again = fx.reopened();
    assert_eq!(again.inner.conversations.len(), 0);
    let second = again
        .start(chat_request(Some(&id), "and more?"))
        .await
        .unwrap();
    assert_eq!(second.conversation_id.as_deref(), Some(id.as_str()));
    finished(&again, &second.run_id).await;

    let request = fx.request(2);
    let messages = request["messages"].as_array().unwrap();
    let roles: Vec<&str> = messages
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    assert_eq!(
        roles,
        ["system", "user", "assistant", "tool", "assistant", "user"]
    );
    assert_eq!(messages[1]["content"], "what is in my box?");
    assert_eq!(
        messages[2]["tool_calls"][0]["function"]["name"],
        "search_emails"
    );
    assert!(
        messages[3]["content"]
            .as_str()
            .unwrap()
            .starts_with("<tool_result")
    );
    assert_eq!(messages[4]["content"], "One mail.");
    // The same conversation, not a copy.
    assert_eq!(fx.list().await.len(), 1);
    assert_eq!(fx.turns(&id).await.len(), 4);
    // An unknown id is still unknown.
    assert_eq!(
        again
            .start(chat_request(Some("missing"), "hi"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
}

#[tokio::test]
async fn a_reply_cut_off_by_a_restart_reads_back_as_stopped_and_can_be_continued() {
    let fx = Fx::new().await;
    // What a crash mid-run leaves: committed rows without a closing row.
    let id = uuid::Uuid::new_v4().to_string();
    let record = ToolRecord {
        activity: ToolActivity {
            call_id: "t".into(),
            name: "search_emails".into(),
            status: ToolStatus::Done,
            summary: ToolActivitySummary::Found { count: 3 },
        },
        confirmation: None,
    };
    let rows = vec![
        history::message_row(
            &ChatMessage::User("look".into()),
            None,
            "2030-01-01T00:00:00.000Z",
        ),
        history::message_row(
            &ChatMessage::Assistant {
                text: "Looking. ".into(),
                tool_calls: vec![crate::ai::client::stream::ToolCallRequest {
                    id: "t".into(),
                    name: "search_emails".into(),
                    arguments: json!({}),
                }],
            },
            None,
            "2030-01-01T00:00:01.000Z",
        ),
        history::message_row(
            &ChatMessage::ToolResult {
                call_id: "t".into(),
                content: "x".into(),
                is_error: false,
            },
            Some(&record),
            "2030-01-01T00:00:02.000Z",
        ),
    ];
    fx.core.with_accounts_for_test(|service| {
        service
            .assistant_chat_append(&crate::storage::assistant_chats::Append {
                id: &id,
                create: Some("look"),
                from_seq: 0,
                rows: &rows,
                accounts: &[],
                now: "2030-01-01T00:00:02.000Z",
            })
            .unwrap()
    });
    let turns = fx.turns(&id).await;
    assert!(matches!(
        &turns[1],
        ChatTurn::Assistant {
            state: TurnState::Canceled,
            finished_reason: None,
            error: None,
            ..
        }
    ));
    fx.model(vec![say("Back again.")]);
    let run = fx
        .assistant
        .start(chat_request(Some(&id), "continue"))
        .await
        .unwrap();
    fx.finished(&run.run_id).await;
    let request = fx.request(0);
    let roles: Vec<&str> = request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect();
    // The interrupted reply is closed with the assistant's stopped note.
    assert_eq!(
        roles,
        ["system", "user", "assistant", "tool", "assistant", "user"]
    );
    assert_eq!(request["messages"][4]["content"], STOPPED_MARKER);
    let turns = fx.turns(&id).await;
    assert_eq!(turns.len(), 4);
    let (text, _, _) = assistant_turn(&turns[1]);
    assert_eq!(text, "Looking. ", "the stopped note is not shown as text");
}

#[tokio::test]
async fn a_canceled_run_is_saved_as_stopped() {
    let fx = Fx::new().await;
    fx.model(vec![say("one")]);
    let first = fx.chat_and_wait("first").await;
    let id = first.conversation_id.unwrap();
    fx.hold(vec![vec![oa_text("par")]]);
    let second = fx
        .assistant
        .start(chat_request(Some(&id), "second"))
        .await
        .unwrap();
    until("text", || {
        fx.assistant.snapshot(&second.run_id).unwrap().text == "par"
    })
    .await;
    fx.assistant.cancel(&second.run_id).unwrap();
    fx.idle(&id).await;
    let turns = fx.turns(&id).await;
    assert_eq!(turns.len(), 4);
    assert!(matches!(
        &turns[3],
        ChatTurn::Assistant {
            state: TurnState::Canceled,
            ..
        }
    ));
    assert!(matches!(&turns[2], ChatTurn::User { text, .. } if text == "second"));
}

#[tokio::test]
async fn a_failed_run_is_saved_with_its_error_code() {
    let fx = Fx::new().await;
    fx.script.lock().unwrap().status = Some(401);
    let run = fx.chat_and_wait("hello").await;
    assert_eq!(run.state, RunState::Failed);
    let turns = fx.turns(&run.conversation_id.unwrap()).await;
    assert!(matches!(
        &turns[1],
        ChatTurn::Assistant {
            state: TurnState::Failed,
            error: Some(ErrorCode::AiKeyRejected),
            ..
        }
    ));
}

#[tokio::test]
async fn the_list_is_newest_first_pages_by_cursor_and_keeps_a_hundred() {
    let fx = Fx::new().await;
    fx.model(vec![say("ok")]);
    let mut ids = Vec::new();
    for n in 0..3 {
        let run = fx.chat_and_wait(&format!("question {n}")).await;
        ids.push(run.conversation_id.unwrap());
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let first = fx
        .assistant
        .list_conversations(Some(2), None)
        .await
        .unwrap();
    let titles: Vec<&str> = first.items.iter().map(|c| c.title.as_str()).collect();
    assert_eq!(titles, ["question 2", "question 1"]);
    let second = fx
        .assistant
        .list_conversations(Some(2), first.next_cursor.clone())
        .await
        .unwrap();
    assert_eq!(second.items.len(), 1);
    assert_eq!(second.items[0].id, ids[0]);
    assert_eq!(second.next_cursor, None);
    for (limit, cursor) in [
        (Some(0), None),
        (Some(101), None),
        (None, Some("junk".to_owned())),
    ] {
        assert_eq!(
            fx.assistant
                .list_conversations(limit, cursor)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Validation
        );
    }

    // The newest hundred stay; a save that pushes past that drops the oldest.
    fx.core.with_accounts_for_test(|service| {
        for n in 0..105 {
            let id = uuid::Uuid::new_v4().to_string();
            let now = format!("2031-01-01T00:00:{:02}.{:03}Z", n / 100, n % 100);
            service
                .assistant_chat_append(&crate::storage::assistant_chats::Append {
                    id: &id,
                    create: Some("filler"),
                    from_seq: 0,
                    rows: &[],
                    accounts: &[],
                    now: &now,
                })
                .unwrap();
        }
    });
    let page = fx
        .assistant
        .list_conversations(Some(MAX_CONVERSATION_PAGE), None)
        .await
        .unwrap();
    assert_eq!(page.items.len(), 100);
    assert_eq!(page.next_cursor, None);
    assert!(page.items.iter().all(|c| c.title == "filler"));
}

#[tokio::test]
async fn deleting_and_clearing_remove_saved_and_memory_copies_and_say_so() {
    let fx = Fx::new().await;
    fx.model(vec![say("ok")]);
    let a = fx.chat_and_wait("a").await.conversation_id.unwrap();
    let b = fx.chat_and_wait("b").await.conversation_id.unwrap();
    fx.assistant.delete_conversation(&a).await.unwrap();
    assert_eq!(
        fx.assistant.get_conversation(&a).await.unwrap_err().code,
        ErrorCode::NotFound
    );
    assert_eq!(
        fx.assistant.delete_conversation(&a).await.unwrap_err().code,
        ErrorCode::NotFound
    );
    assert_eq!(
        fx.assistant
            .start(chat_request(Some(&a), "more"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    assert_eq!(fx.list().await.len(), 1);
    assert!(fx.stored_contents(&a).is_empty());
    fx.assistant.clear_conversations().await.unwrap();
    assert!(fx.list().await.is_empty());
    assert_eq!(fx.stored_rows(), 0);
    assert_eq!(
        fx.assistant
            .start(chat_request(Some(&b), "more"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    let changes: Vec<(Option<String>, ConversationChange)> = fx
        .events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            CoreEvent::AssistantConversationsChanged(c) => {
                Some((c.conversation_id.clone(), c.change))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        changes,
        [
            (Some(a.clone()), ConversationChange::Saved),
            (Some(b.clone()), ConversationChange::Saved),
            (Some(a), ConversationChange::Deleted),
            (None, ConversationChange::Cleared),
        ]
    );
}

#[tokio::test]
async fn deleting_a_conversation_cancels_its_run_and_nothing_brings_it_back() {
    let fx = Fx::new().await;
    fx.model(vec![say("one")]);
    let id = fx.chat_and_wait("first").await.conversation_id.unwrap();
    fx.hold(vec![vec![oa_text("par")]]);
    let run = fx
        .assistant
        .start(chat_request(Some(&id), "second"))
        .await
        .unwrap();
    until("text", || {
        fx.assistant.snapshot(&run.run_id).unwrap().text == "par"
    })
    .await;
    fx.assistant.delete_conversation(&id).await.unwrap();
    assert_eq!(
        fx.assistant.snapshot(&run.run_id).unwrap().state,
        RunState::Canceled
    );
    // Let the canceled run's own save come and go.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(fx.list().await.is_empty());
    assert_eq!(fx.stored_rows(), 0);
    assert_eq!(fx.assistant.inner.conversations.len(), 0);
}

#[tokio::test]
async fn with_history_off_nothing_new_is_saved_and_what_was_saved_stays() {
    let fx = Fx::new().await;
    fx.model(vec![say("ok")]);
    let id = fx.chat_and_wait("kept").await.conversation_id.unwrap();
    let rows = fx.stored_rows();
    fx.configure(|a| a.save_history = false);

    // A new conversation is not saved at all ...
    let other = fx.chat_and_wait("private").await.conversation_id.unwrap();
    assert_eq!(fx.list().await.len(), 1);
    // ... a saved one gets no new turns ...
    let more = fx
        .assistant
        .start(chat_request(Some(&id), "private too"))
        .await
        .unwrap();
    fx.finished(&more.run_id).await;
    assert_eq!(fx.stored_rows(), rows);
    // ... and turning history on again does not save what ran while it was off.
    fx.configure(|a| a.save_history = true);
    let again = fx
        .assistant
        .start(chat_request(Some(&other), "still private"))
        .await
        .unwrap();
    fx.finished(&again.run_id).await;
    assert_eq!(fx.stored_rows(), rows);
    assert_eq!(fx.list().await.len(), 1);
    // The working copy still answers with its history.
    assert_eq!(fx.requests(), 4);
    assert_eq!(fx.request(3)["messages"].as_array().unwrap().len(), 4);
    // New conversations save again.
    fx.chat_and_wait("public").await;
    assert_eq!(fx.list().await.len(), 2);
}

#[tokio::test]
async fn removing_an_account_deletes_the_conversations_that_touched_it_and_only_those() {
    let fx = Fx::new().await;
    let other = fx.other_account();
    fx.model(vec![
        calls(&[("s", "search_emails", json!({"query": "hello"}))]),
        say("Here."),
        calls(&[("f", "list_folders", json!({"accountId": other}))]),
        say("No folders."),
        say("Just chatting."),
    ]);
    let about_mail = fx.chat_and_wait("search").await.conversation_id.unwrap();
    let about_other = fx
        .chat_and_wait("folders of the other")
        .await
        .conversation_id
        .unwrap();
    let plain = fx.chat_and_wait("hello").await.conversation_id.unwrap();
    let linked = |id: &str| -> Vec<String> {
        let mut statement = fx
            .db
            .prepare(
                "SELECT account_id FROM assistant_conversation_accounts
                 WHERE conversation_id = ?1 ORDER BY 1",
            )
            .unwrap();
        statement
            .query_map([id], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(linked(&about_mail), std::slice::from_ref(&fx.account));
    assert_eq!(linked(&about_other), std::slice::from_ref(&other));
    assert!(linked(&plain).is_empty());

    fx.core
        .remove_account(crate::RemoveAccountRequest {
            account_id: other.clone(),
            expected_revision: 1,
        })
        .unwrap();
    let left: Vec<String> = fx.list().await.into_iter().map(|c| c.id).collect();
    assert!(left.contains(&about_mail) && left.contains(&plain));
    assert!(!left.contains(&about_other));
    assert!(fx.stored_contents(&about_other).is_empty());
    assert!(linked(&about_other).is_empty());
    // The working copy of the removed conversation cannot be continued.
    assert_eq!(
        fx.assistant
            .start(chat_request(Some(&about_other), "more"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    assert!(fx.events.lock().unwrap().iter().any(|e| matches!(
        e,
        CoreEvent::AssistantConversationsChanged(c) if c.change == ConversationChange::Cleared
    )));
}

#[test]
fn a_conversation_with_four_hundred_messages_is_full() {
    let store = Arc::new(Conversations::default());
    let (guard, _) = store.begin(None, "m".into()).unwrap();
    let id = guard.id.clone();
    let messages: Vec<ChatMessage> = (0..conversations::MAX_MESSAGES)
        .map(|n| {
            if n % 2 == 0 {
                ChatMessage::User(format!("q{n}"))
            } else {
                ChatMessage::Assistant {
                    text: "a".into(),
                    tool_calls: Vec::new(),
                }
            }
        })
        .collect();
    store.commit(&id, &messages, &conversations::RunExtras::default());
    drop(guard);
    assert_eq!(
        store
            .begin(Some(&id), "one more".into())
            .err()
            .map(|e| e.code),
        Some(ErrorCode::AssistantContextFull)
    );
}
