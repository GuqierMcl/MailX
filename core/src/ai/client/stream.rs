//! Streaming chat with tool calls over the two wire protocols. Provider text
//! never leaves this module as text: only parsed fields (model output, tool
//! names and arguments) and fixed error codes do. Dropping the future of
//! [`stream_chat`] closes the connection; that is how a run is cancelled.
//!
//! A stream that ends without a finish marker (`[DONE]` / `message_stop`) is
//! `AiInvalidResponse` when nothing useful arrived, and a normal
//! `Finished { reason: Other }` when text, a tool call or a finish reason did:
//! some servers simply close the connection after the last chunk, and the
//! caller already has everything the model said.

mod anthropic;
mod openai;
mod sse;

use hyper::Method;
use serde_json::Value;

use super::{Endpoint, auth_headers, http_error, status_error, url};
use crate::{
    ErrorCode,
    ai::{
        reasoning::{AssistantReasoning, ReasoningEcho, ReasoningOrigin},
        settings::AiProtocol,
    },
    net::http::{self, HttpRequest, StreamLimits},
};

/// Streaming replies get room for a tool call's arguments (a drafted mail) and
/// a normal answer; the non-streaming default of 1024 would cut them off and
/// leave invalid tool JSON. Every current Claude model allows at least 4096.
const ANTHROPIC_STREAM_DEFAULT_MAX_TOKENS: u32 = 4096;

/// The limits for a model turn: first piece 30 s, 60 s between pieces (any
/// bytes count, reasoning included) and 10 minutes in all, so a model that
/// thinks at length while it keeps streaming is not cut off.
pub(crate) const CHAT_STREAM_LIMITS: StreamLimits = StreamLimits {
    first_byte: std::time::Duration::from_secs(30),
    idle: std::time::Duration::from_secs(60),
    total: std::time::Duration::from_secs(10 * 60),
    max_bytes: 8 * 1024 * 1024,
};

pub(crate) struct ChatRequest<'a> {
    pub model: &'a str,
    pub system: Option<&'a str>,
    pub messages: &'a [ChatMessage],
    pub tools: &'a [ToolDefinition],
    /// `None` sends no cap on OpenAI-compatible servers; Anthropic always gets
    /// one.
    pub max_output: Option<u32>,
    /// Who answers this request: stored reasoning goes back only to the
    /// server and model that produced it.
    pub origin: &'a ReasoningOrigin,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ChatMessage {
    User(String),
    Assistant {
        text: String,
        tool_calls: Vec<ToolCallRequest>,
        /// What the model reasoned in this turn, kept to be sent back.
        reasoning: Option<AssistantReasoning>,
    },
    ToolResult {
        call_id: String,
        content: String,
        is_error: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ToolDefinition {
    pub name: String,
    pub description: String,
    /// A JSON Schema object.
    pub input_schema: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ToolCallRequest {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum StreamEvent {
    Text(String),
    /// Readable reasoning as it streams. Empty when a reasoning block starts
    /// without text (redacted thinking, or a block that fills in later).
    Reasoning(String),
    /// Everything to send back for this reply's reasoning, once, before
    /// `Finished`; absent when the server sent none.
    ReasoningEcho(ReasoningEcho),
    /// Whole: the arguments are complete, valid JSON.
    ToolCall(ToolCallRequest),
    Finished {
        reason: FinishReason,
        usage: Option<Usage>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FinishReason {
    Stop,
    ToolCalls,
    Length,
    Other,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Usage {
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
}

/// Turns SSE events into [`StreamEvent`]s.
trait Decoder {
    /// Handles one event; `Ok(true)` means the stream is complete.
    fn event(
        &mut self,
        event: &sse::SseEvent,
        out: &mut Vec<StreamEvent>,
    ) -> Result<bool, ErrorCode>;
    /// The body ended before the protocol's end marker.
    fn end(&mut self, out: &mut Vec<StreamEvent>) -> Result<(), ErrorCode>;
}

fn token_count(value: Option<&Value>) -> Option<u32> {
    value
        .and_then(Value::as_u64)
        .map(|n| u32::try_from(n).unwrap_or(u32::MAX))
}

/// Tool arguments as JSON: an empty string is `{}`; anything else must parse.
fn parse_arguments(text: &str) -> Result<Value, ErrorCode> {
    if text.trim().is_empty() {
        return Ok(Value::Object(serde_json::Map::new()));
    }
    serde_json::from_str(text).map_err(|_| ErrorCode::AiInvalidResponse)
}

/// One streaming request. `on_event` gets the events in order: text pieces as
/// they arrive, each tool call once complete, then `Finished`. Errors after
/// some events were delivered are returned as usual; the caller keeps what it
/// received. Non-2xx statuses and transport failures use the same fixed codes
/// as the non-streaming calls; the limits bound the answer (idle gap
/// and total time give `AiTimeout`, size gives `AiInvalidResponse`).
pub(crate) async fn stream_chat(
    endpoint: &Endpoint<'_>,
    request: &ChatRequest<'_>,
    limits: StreamLimits,
    mut on_event: impl FnMut(StreamEvent),
) -> Result<(), ErrorCode> {
    let (suffix, body, mut decoder): (_, _, Box<dyn Decoder + Send>) = match endpoint.protocol {
        AiProtocol::OpenaiCompatible => (
            "/chat/completions",
            openai::body(request),
            Box::<openai::OpenAiDecoder>::default(),
        ),
        AiProtocol::Anthropic => (
            "/messages",
            anthropic::body(request),
            Box::<anthropic::AnthropicDecoder>::default(),
        ),
    };
    let url = url(endpoint, suffix)?;
    let mut reply = http::send_streaming(
        HttpRequest {
            method: Method::POST,
            url: &url,
            route: endpoint.route,
            headers: auth_headers(endpoint)?,
            body: Some(serde_json::to_vec(&body).unwrap_or_default()),
            timeout: limits.total,
            max_response_bytes: limits.max_bytes,
        },
        limits,
    )
    .await
    .map_err(http_error)?;
    if !(200..300).contains(&reply.status) {
        return Err(status_error(reply.status, true));
    }

    let mut parser = sse::SseParser::default();
    let mut events = Vec::new();
    let mut out = Vec::new();
    let mut done = false;
    while !done {
        let Some(chunk) = reply.next_chunk().await.map_err(http_error)? else {
            break;
        };
        parser.push(&chunk, &mut events);
        done = decode(&mut *decoder, &mut events, &mut out, &mut on_event)?;
    }
    if !done {
        parser.finish(&mut events);
        done = decode(&mut *decoder, &mut events, &mut out, &mut on_event)?;
    }
    if !done {
        let ended = decoder.end(&mut out);
        out.drain(..).for_each(&mut on_event);
        ended?;
    }
    Ok(())
}

/// Runs the decoder over `events`, delivering what it produced even when it
/// then fails.
fn decode(
    decoder: &mut dyn Decoder,
    events: &mut Vec<sse::SseEvent>,
    out: &mut Vec<StreamEvent>,
    on_event: &mut impl FnMut(StreamEvent),
) -> Result<bool, ErrorCode> {
    let mut result = Ok(false);
    for event in events.drain(..) {
        result = decoder.event(&event, out);
        if !matches!(result, Ok(false)) {
            break;
        }
    }
    out.drain(..).for_each(&mut *on_event);
    result
}

#[cfg(test)]
mod tests;
