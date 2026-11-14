//! The two run bodies: the chat loop (model turn, tool calls, results, again)
//! and the Now summary (one tool-less model turn). Limits are
//! `docs/ai-runtime/README.md` section 8. Provider text, prompts and mail
//! content are never logged and never leave in events: events carry model
//! output, structured tool summaries and fixed error codes only.
use std::sync::Arc;

use serde_json::Value;

use super::{
    Gate, Inner, Ready,
    conversations::ActiveConversation,
    policy::{self, Decision, DraftSource},
    prompt,
    summary::MAX_OUTPUT_TOKENS,
    types::{FinishedReason, ToolActivitySummary, ToolStatus},
};
use crate::{
    ErrorCode,
    ai::client::{
        Endpoint,
        stream::{
            CHAT_STREAM_LIMITS, ChatMessage, ChatRequest, FinishReason, StreamEvent,
            ToolCallRequest, ToolDefinition, stream_chat,
        },
    },
    tools::{Audience, Principal, RequestContext, Tools},
};

/// Model requests for one user message.
pub(crate) const MAX_TURNS: usize = 20;
/// Tool calls run per model turn; more get an error result.
pub(crate) const MAX_TOOL_CALLS_PER_TURN: usize = 8;
/// Assumed context window when the catalog does not know the model.
pub(crate) const DEFAULT_CONTEXT_TOKENS: u32 = 32_000;
/// A conversation stops growing at this share of the window.
const CONTEXT_SHARE_PERCENT: u64 = 80;
/// Longest tool result handed to the model, in characters.
const MAX_RESULT_CHARS: usize = 64_000;
const MAX_SUMMARY_ITEMS: usize = 50;

enum Outcome {
    Finished(FinishedReason),
    Failed(ErrorCode),
    NeedsConsent,
}

/// Ends a run that is still active when its task ends any way other than
/// reporting (a panic): a run must never stay `running` forever. After a
/// normal finish or a cancel the run is over and this does nothing.
struct EndGuard {
    inner: Arc<Inner>,
    run_id: String,
}

impl Drop for EndGuard {
    fn drop(&mut self) {
        self.inner.runs.fail(&self.run_id, ErrorCode::Internal);
    }
}

fn report(inner: &Inner, run_id: &str, outcome: Outcome) {
    match outcome {
        Outcome::Finished(reason) => inner.runs.complete(run_id, reason),
        Outcome::Failed(code) => inner.runs.fail(run_id, code),
        Outcome::NeedsConsent => inner.runs.await_consent(run_id),
    }
}

// ---------------------------------------------------------------- estimates

/// Tokens of a text: about four characters each, one per character beyond
/// ASCII (CJK is far denser than that).
fn estimate(text: &str) -> u64 {
    let (ascii, other) = text.chars().fold((0u64, 0u64), |(a, o), c| {
        if c.is_ascii() { (a + 1, o) } else { (a, o + 1) }
    });
    ascii.div_ceil(4) + other
}

fn estimate_messages(messages: &[ChatMessage]) -> u64 {
    messages
        .iter()
        .map(|message| match message {
            ChatMessage::User(text) => estimate(text),
            ChatMessage::Assistant { text, tool_calls } => {
                estimate(text)
                    + tool_calls
                        .iter()
                        .map(|call| estimate(&call.name) + estimate(&call.arguments.to_string()))
                        .sum::<u64>()
            }
            ChatMessage::ToolResult { content, .. } => estimate(content),
        })
        .sum()
}

fn budget(target: &crate::ai::AiModelTarget) -> u64 {
    u64::from(target.context_length.unwrap_or(DEFAULT_CONTEXT_TOKENS)) * CONTEXT_SHARE_PERCENT / 100
}

// ------------------------------------------------------------------- stream

#[derive(Default)]
struct Turn {
    text: String,
    calls: Vec<ToolCallRequest>,
    reason: Option<FinishReason>,
}

/// One streaming request. Text goes to the run as it arrives; what the model
/// said before a failure stays in the run's snapshot.
async fn model_turn(
    inner: &Inner,
    run_id: &str,
    ready: &Ready,
    system: &str,
    messages: &[ChatMessage],
    tools: &[ToolDefinition],
    max_output: Option<u32>,
) -> Result<Turn, ErrorCode> {
    let target = &ready.target;
    let endpoint = Endpoint {
        protocol: target.protocol,
        base_url: &target.binding.base_url,
        key: Some(target.key.expose_secret()),
        route: &target.route,
    };
    let request = ChatRequest {
        model: &target.model_id,
        system: Some(system),
        messages,
        tools,
        max_output,
    };
    let mut turn = Turn::default();
    stream_chat(
        &endpoint,
        &request,
        CHAT_STREAM_LIMITS,
        |event| match event {
            StreamEvent::Text(delta) => {
                turn.text.push_str(&delta);
                inner.runs.text(run_id, delta);
            }
            StreamEvent::ToolCall(call) => turn.calls.push(call),
            StreamEvent::Finished { reason, .. } => turn.reason = Some(reason),
        },
    )
    .await?;
    Ok(turn)
}

// --------------------------------------------------------------------- chat

pub(crate) async fn chat(
    inner: Arc<Inner>,
    run_id: String,
    conversation: ActiveConversation,
    mut messages: Vec<ChatMessage>,
) {
    let _end = EndGuard {
        inner: inner.clone(),
        run_id: run_id.clone(),
    };
    let outcome = chat_loop(&inner, &run_id, &conversation.id, &mut messages).await;
    report(&inner, &run_id, outcome);
}

fn definitions(tools: &Tools) -> Vec<ToolDefinition> {
    tools
        .list(Audience::BuiltinAgent)
        .into_iter()
        .map(|tool| ToolDefinition {
            name: tool.id.to_owned(),
            description: tool.description.to_owned(),
            input_schema: tool.input_schema,
        })
        .collect()
}

fn utc_now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

async fn chat_loop(
    inner: &Arc<Inner>,
    run_id: &str,
    conversation_id: &str,
    messages: &mut Vec<ChatMessage>,
) -> Outcome {
    let definitions = definitions(&inner.tools);
    let definitions_cost = estimate(
        &serde_json::to_string(
            &definitions
                .iter()
                .map(|d| (&d.name, &d.description, &d.input_schema))
                .collect::<Vec<_>>(),
        )
        .unwrap_or_default(),
    );
    for turn_number in 1..=MAX_TURNS {
        // Settings are read again every turn: a switched-off assistant, a
        // revoked consent or another model applies at once.
        let ready = match inner.gate().await {
            Ok(Gate::Ready(ready)) => ready,
            Ok(Gate::NeedsConsent) => return Outcome::NeedsConsent,
            Err(error) => return Outcome::Failed(error.code),
        };
        let system = prompt::chat_system(ready.locale, &utc_now(), &ready.settings.instructions);
        if estimate(&system) + definitions_cost + estimate_messages(messages)
            > budget(&ready.target)
        {
            return Outcome::Failed(ErrorCode::AssistantContextFull);
        }
        let turn = match model_turn(
            inner,
            run_id,
            &ready,
            &system,
            messages,
            &definitions,
            ready.target.output_length,
        )
        .await
        {
            Ok(turn) => turn,
            Err(code) => return Outcome::Failed(code),
        };
        messages.push(ChatMessage::Assistant {
            text: turn.text,
            tool_calls: turn.calls.clone(),
        });
        if turn.calls.is_empty() {
            inner.conversations.commit(conversation_id, messages);
            return Outcome::Finished(if turn.reason == Some(FinishReason::Length) {
                FinishedReason::Length
            } else {
                FinishedReason::Stop
            });
        }
        let last_turn = turn_number == MAX_TURNS;
        for (index, call) in turn.calls.iter().enumerate() {
            let result = if last_turn || index >= MAX_TOOL_CALLS_PER_TURN {
                Executed::failed("limit", "Too many tool calls. Nothing was run.")
            } else {
                inner.runs.tool(
                    run_id,
                    &call.id,
                    &call.name,
                    ToolStatus::Running,
                    ToolActivitySummary::Pending,
                );
                run_tool(inner, run_id, call).await
            };
            inner
                .runs
                .tool(run_id, &call.id, &call.name, result.status, result.summary);
            messages.push(ChatMessage::ToolResult {
                call_id: call.id.clone(),
                content: result.content,
                is_error: result.is_error,
            });
        }
        // The turn is whole again: assistant message and all its results.
        inner.conversations.commit(conversation_id, messages);
    }
    Outcome::Finished(FinishedReason::Limit)
}

// -------------------------------------------------------------------- tools

struct Executed {
    status: ToolStatus,
    summary: ToolActivitySummary,
    content: String,
    is_error: bool,
}

impl Executed {
    fn failed(code: &str, message: &str) -> Self {
        Self {
            status: ToolStatus::Failed,
            summary: ToolActivitySummary::Error {
                code: code.to_owned(),
            },
            content: format!("Error {code}: {message}"),
            is_error: true,
        }
    }

    /// The user's settings ask first and nobody can answer yet, so this is a
    /// refusal the model is told about.
    fn denied() -> Self {
        Self {
            status: ToolStatus::Denied,
            summary: ToolActivitySummary::Denied,
            content: "The user declined this action. It was not run. Do not try again unless \
                      the user asks you to."
                .to_owned(),
            is_error: true,
        }
    }
}

/// Tool code is synchronous and blocks on storage, so it runs on a blocking
/// thread. Dropping this future (a cancel) abandons the wait; a call already
/// running finishes and its result is dropped.
async fn run_tool(inner: &Arc<Inner>, run_id: &str, call: &ToolCallRequest) -> Executed {
    let inner = inner.clone();
    let run_id = run_id.to_owned();
    let call = call.clone();
    tokio::task::spawn_blocking(move || execute(&inner, &run_id, &call))
        .await
        .unwrap_or_else(|_| Executed::failed("internal", "An internal error occurred."))
}

fn draft_source(inner: &Inner, arguments: &Value, grant: &crate::tools::Grant) -> DraftSource {
    let Some(id) = arguments.get("draftId").and_then(Value::as_str) else {
        // The tool rejects the input itself.
        return DraftSource::Missing;
    };
    match inner.core.get_draft(id) {
        Ok(draft) if grant.accounts.permits(&draft.account_id) => DraftSource::of(draft.origin),
        Ok(_) => DraftSource::Missing,
        Err(error) if matches!(error.code, ErrorCode::Validation | ErrorCode::NotFound) => {
            DraftSource::Missing
        }
        Err(_) => DraftSource::Unknown,
    }
}

fn execute(inner: &Inner, run_id: &str, call: &ToolCallRequest) -> Executed {
    let Ok(snapshot) = inner.settings.get() else {
        return Executed::failed("unavailable", "Settings could not be read.");
    };
    let settings = snapshot.settings.assistant;
    if !settings.enabled {
        return Executed::failed("notPermitted", "The assistant is turned off.");
    }
    // The ids only matter when some account is left out.
    let ids: Vec<String> = if settings.excluded_accounts.is_empty() {
        Vec::new()
    } else {
        match inner.core.account_identities() {
            Ok(identities) => identities.into_iter().map(|a| a.id).collect(),
            Err(_) => return Executed::failed("unavailable", "MailX is temporarily unavailable."),
        }
    };
    let grant = policy::grant(&settings, &ids);
    let source = if policy::needs_draft(&call.name) {
        draft_source(inner, &call.arguments, &grant)
    } else {
        DraftSource::Missing
    };
    if policy::classify(&call.name, &settings, source) == Decision::Ask {
        // The confirmation flow does not exist yet: asking means declined.
        return Executed::denied();
    }
    let context = RequestContext::new(
        Principal::BuiltinAgent {
            run_id: run_id.to_owned(),
        },
        grant,
        call.id.chars().take(64).collect::<String>(),
    );
    match inner
        .tools
        .call(&context, &call.name, call.arguments.clone())
    {
        Ok(output) => {
            let text = serde_json::to_string(&output).unwrap_or_default();
            let text: String = text.chars().take(MAX_RESULT_CHARS).collect();
            Executed {
                status: ToolStatus::Done,
                summary: summarize(&call.name, &call.arguments, &output),
                content: prompt::wrap_result(&call.name, &text),
                is_error: false,
            }
        }
        Err(error) => {
            let code = serde_json::to_value(error.code)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_else(|| "internal".into());
            // Both are fixed strings of the tool layer, never mail or server text.
            Executed::failed(&code, &error.message)
        }
    }
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .take(MAX_SUMMARY_ITEMS)
                .map(|id| id.chars().take(64).collect())
                .collect()
        })
        .unwrap_or_default()
}

fn count(value: Option<&Value>) -> u32 {
    value
        .and_then(Value::as_array)
        .map_or(0, |items| u32::try_from(items.len()).unwrap_or(u32::MAX))
}

fn text(output: &Value, field: &str, max: usize) -> String {
    output
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .chars()
        .take(max)
        .collect()
}

/// A short structured account of a finished call, from the tool's input and
/// output. Never raw output.
fn summarize(name: &str, input: &Value, output: &Value) -> ToolActivitySummary {
    match name {
        "search_emails" => ToolActivitySummary::Found {
            count: count(output.get("emails")),
        },
        "list_accounts" => ToolActivitySummary::Found {
            count: count(output.get("accounts")),
        },
        "list_folders" => ToolActivitySummary::Found {
            count: count(output.get("folders")),
        },
        "list_drafts" => ToolActivitySummary::Found {
            count: count(output.get("drafts")),
        },
        "list_outbox" => ToolActivitySummary::Found {
            count: count(output.get("items")),
        },
        "read_email" => ToolActivitySummary::Read {
            email_ids: input
                .get("emailId")
                .and_then(Value::as_str)
                .map(|id| vec![id.chars().take(64).collect()])
                .unwrap_or_default(),
        },
        "set_email_flags" => {
            let changed: Vec<String> = output
                .get("results")
                .and_then(Value::as_array)
                .map(|results| {
                    results
                        .iter()
                        .filter(|result| {
                            ["isRead", "isStarred"].iter().any(|field| {
                                result
                                    .get(*field)
                                    .and_then(|r| r.get("status"))
                                    .and_then(Value::as_str)
                                    .is_some_and(|s| s == "queued" || s == "localOnly")
                            })
                        })
                        .filter_map(|result| result.get("emailId").and_then(Value::as_str))
                        .take(MAX_SUMMARY_ITEMS)
                        .map(|id| id.chars().take(64).collect())
                        .collect()
                })
                .unwrap_or_default();
            ToolActivitySummary::Changed {
                count: u32::try_from(changed.len()).unwrap_or(u32::MAX),
                email_ids: changed,
            }
        }
        "archive_emails" | "unarchive_emails" => ToolActivitySummary::Changed {
            count: output
                .get("changedCount")
                .and_then(Value::as_u64)
                .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX)),
            email_ids: strings(input.get("emailIds")),
        },
        "create_draft" | "create_reply_draft" => ToolActivitySummary::Draft {
            draft_id: text(output, "draftId", 64),
            subject: text(output, "subject", 200),
        },
        "update_draft" => ToolActivitySummary::Draft {
            draft_id: text(output, "draftId", 64),
            subject: text(output, "subject", 200),
        },
        "send_email" | "get_outbox" | "cancel_outbox" => ToolActivitySummary::Outbox {
            outbox_id: text(output, "id", 64),
        },
        _ => ToolActivitySummary::Done,
    }
}

// ------------------------------------------------------------------ summary

pub(crate) async fn summary(inner: Arc<Inner>, run_id: String, input: String, key: String) {
    let _end = EndGuard {
        inner: inner.clone(),
        run_id: run_id.clone(),
    };
    let outcome = summary_turn(&inner, &run_id, input, key).await;
    report(&inner, &run_id, outcome);
}

async fn summary_turn(inner: &Arc<Inner>, run_id: &str, input: String, key: String) -> Outcome {
    let ready = match inner.gate().await {
        Ok(Gate::Ready(ready)) => ready,
        Ok(Gate::NeedsConsent) => return Outcome::NeedsConsent,
        Err(error) => return Outcome::Failed(error.code),
    };
    let system = prompt::summary_system(ready.locale);
    let messages = [ChatMessage::User(input)];
    let max_output = ready
        .target
        .output_length
        .map_or(MAX_OUTPUT_TOKENS, |n| n.min(MAX_OUTPUT_TOKENS));
    let turn = match model_turn(
        inner,
        run_id,
        &ready,
        &system,
        &messages,
        &[],
        Some(max_output),
    )
    .await
    {
        Ok(turn) => turn,
        Err(code) => return Outcome::Failed(code),
    };
    let answer = turn.text.trim().to_owned();
    if answer.is_empty() {
        return Outcome::Failed(ErrorCode::AiInvalidResponse);
    }
    let cut = turn.reason == Some(FinishReason::Length);
    if !cut {
        inner.summaries.put(key, answer);
    }
    Outcome::Finished(if cut {
        FinishedReason::Length
    } else {
        FinishedReason::Stop
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn estimates_count_cjk_densely() {
        assert_eq!(estimate("abcdefgh"), 2);
        assert_eq!(estimate("你好"), 2);
        assert_eq!(estimate(""), 0);
    }

    #[test]
    fn summaries_are_structured_and_bounded() {
        let found = summarize(
            "search_emails",
            &json!({}),
            &json!({"emails": [{"id": "a"}, {"id": "b"}]}),
        );
        assert_eq!(found, ToolActivitySummary::Found { count: 2 });
        let changed = summarize(
            "archive_emails",
            &json!({"emailIds": ["a", "b"]}),
            &json!({"processedCount": 2, "changedCount": 1}),
        );
        assert_eq!(
            changed,
            ToolActivitySummary::Changed {
                count: 1,
                email_ids: vec!["a".into(), "b".into()]
            }
        );
        let flags = summarize(
            "set_email_flags",
            &json!({}),
            &json!({"results": [
                {"emailId": "a", "isRead": {"status": "queued"}},
                {"emailId": "b", "isRead": {"status": "unchanged"}},
                {"emailId": "c", "isStarred": {"status": "failed", "errorCode": "notFound"}},
            ]}),
        );
        assert_eq!(
            flags,
            ToolActivitySummary::Changed {
                count: 1,
                email_ids: vec!["a".into()]
            }
        );
        let draft = summarize(
            "create_draft",
            &json!({}),
            &json!({"draftId": "d1", "subject": "Hi", "accountId": "x"}),
        );
        assert_eq!(
            draft,
            ToolActivitySummary::Draft {
                draft_id: "d1".into(),
                subject: "Hi".into()
            }
        );
        assert_eq!(
            summarize("refresh_sync", &json!({}), &json!({"accepted": true})),
            ToolActivitySummary::Done
        );
    }
}
