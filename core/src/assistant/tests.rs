//! Runs against a scripted streaming model and a real test core.
use std::{
    sync::{Arc, Mutex, atomic::Ordering},
    time::{Duration, Instant},
};

use rusqlite::{Connection, params};
use serde_json::{Value, json};
use tempfile::TempDir;

use super::{
    conversations::{Conversations, MAX_CONVERSATIONS, STOPPED_MARKER},
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

#[tokio::test]
async fn organize_tools_run_when_direct_and_are_denied_when_asking() {
    let args = |fx: &Fx| json!({"emailIds": [fx.emails[0]], "isRead": true});
    // Direct (the default).
    let fx = Fx::new().await;
    assert!(!is_read(&fx, 0));
    fx.model(vec![
        calls(&[("c1", "set_email_flags", args(&fx))]),
        say("Marked."),
    ]);
    let snapshot = fx.chat_and_wait("mark it read").await;
    assert_eq!(snapshot.tools[0].status, ToolStatus::Done);
    assert!(matches!(
        snapshot.tools[0].summary,
        ToolActivitySummary::Changed { count: 1, .. }
    ));
    assert!(is_read(&fx, 0));

    // Ask: not run, and the model is told.
    let fx = Fx::new().await;
    fx.configure(|a| a.organize = AssistantMode::Ask);
    fx.model(vec![
        calls(&[("c1", "set_email_flags", args(&fx))]),
        say("I could not."),
    ]);
    let snapshot = fx.chat_and_wait("mark it read").await;
    assert_eq!(snapshot.state, RunState::Completed);
    assert_eq!(snapshot.tools[0].status, ToolStatus::Denied);
    assert_eq!(snapshot.tools[0].summary, ToolActivitySummary::Denied);
    assert!(!is_read(&fx, 0));
    let results = tool_messages(&fx.request(1));
    assert!(results[0].contains("declined"), "{results:?}");
    assert!(!results[0].contains("tool_result"), "not tool output");
}

#[tokio::test]
async fn sending_is_denied_in_confirm_mode_and_queues_in_direct_mode() {
    let outbox_events = |fx: &Fx| {
        fx.events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| matches!(e, CoreEvent::Outbox(_)))
            .count()
    };
    let send = |draft: &crate::Draft| {
        json!({"action": "send", "draftId": draft.id, "expectedRevision": draft.revision,
               "clientMutationId": uuid::Uuid::new_v4().to_string()})
    };

    let fx = Fx::new().await;
    let draft = fx.user_draft();
    fx.model(vec![
        calls(&[("c1", "send_email", send(&draft))]),
        say("Not sent."),
    ]);
    let snapshot = fx.chat_and_wait("send my draft").await;
    assert_eq!(snapshot.tools[0].status, ToolStatus::Denied);
    assert_eq!(outbox_events(&fx), 0, "nothing was queued");
    assert!(fx.core.list_outbox(None, true).unwrap().is_empty());

    let fx = Fx::new().await;
    fx.configure(|a| a.send = AssistantSendMode::Direct);
    let draft = fx.user_draft();
    fx.model(vec![
        calls(&[("c1", "send_email", send(&draft))]),
        say("Queued."),
    ]);
    let snapshot = fx.chat_and_wait("send my draft").await;
    assert_eq!(snapshot.tools[0].status, ToolStatus::Done);
    let ToolActivitySummary::Outbox { outbox_id } = &snapshot.tools[0].summary else {
        panic!("{:?}", snapshot.tools[0].summary);
    };
    let item = fx.core.get_outbox(outbox_id).unwrap();
    assert_eq!(item.draft_id.as_deref(), Some(draft.id.as_str()));
}

#[tokio::test]
async fn the_assistants_own_drafts_follow_ai_drafts_and_the_users_are_always_asked() {
    let fx = Fx::new().await;
    fx.model(vec![
        calls(&[(
            "c1",
            "create_draft",
            json!({"accountId": fx.account, "to": [{"address": "a@example.test"}],
                   "subject": "From AI", "bodyHtml": "<p>x</p>"}),
        )]),
        say("Drafted."),
    ]);
    let snapshot = fx.chat_and_wait("draft something").await;
    let ToolActivitySummary::Draft { draft_id, subject } = &snapshot.tools[0].summary else {
        panic!("{:?}", snapshot.tools[0]);
    };
    assert_eq!(subject, "From AI");
    let ai_draft = fx.core.get_draft(draft_id).unwrap();
    assert_eq!(ai_draft.origin, crate::DraftOrigin::Ai);
    let user_draft = fx.user_draft();

    let update = |draft: &crate::Draft| json!({"draftId": draft.id, "expectedRevision": draft.revision, "subject": "Changed"});
    // Everything direct: still only the assistant's own draft changes.
    fx.configure(|a| {
        a.organize = AssistantMode::Direct;
        a.ai_drafts = AssistantMode::Direct;
        a.send = AssistantSendMode::Direct;
    });
    fx.model(vec![
        calls(&[
            ("u1", "update_draft", update(&ai_draft)),
            ("u2", "update_draft", update(&user_draft)),
        ]),
        say("Done."),
    ]);
    let snapshot = fx.chat_and_wait("rename both").await;
    let status = |id: &str| {
        snapshot
            .tools
            .iter()
            .find(|t| t.call_id == id)
            .unwrap()
            .status
    };
    assert_eq!(status("u1"), ToolStatus::Done);
    assert_eq!(status("u2"), ToolStatus::Denied);
    assert_eq!(fx.core.get_draft(&ai_draft.id).unwrap().subject, "Changed");
    assert_eq!(
        fx.core.get_draft(&user_draft.id).unwrap().subject,
        "User draft"
    );

    // aiDrafts = ask: the assistant's own draft is asked about too.
    fx.configure(|a| a.ai_drafts = AssistantMode::Ask);
    let current = fx.core.get_draft(&ai_draft.id).unwrap();
    fx.model(vec![
        calls(&[("u3", "update_draft", update(&current))]),
        say("Done."),
    ]);
    let snapshot = fx.chat_and_wait("rename again").await;
    assert_eq!(snapshot.tools[0].status, ToolStatus::Denied);
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
    // An unknown tool is never run: the policy asks, and asking is declined.
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
        fx.assistant
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
