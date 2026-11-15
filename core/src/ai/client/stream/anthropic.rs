//! Anthropic Messages: request body and stream decoding.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::{
    ANTHROPIC_STREAM_DEFAULT_MAX_TOKENS, ChatMessage, ChatRequest, Decoder, FinishReason,
    StreamEvent, ToolCallRequest, Usage, parse_arguments, sse::SseEvent, token_count,
};
use crate::ErrorCode;

pub(super) fn body(request: &ChatRequest<'_>) -> Value {
    let mut messages: Vec<Value> = Vec::new();
    // Tool results for one assistant turn share a single user message.
    let mut results: Vec<Value> = Vec::new();
    let flush = |messages: &mut Vec<Value>, results: &mut Vec<Value>| {
        if !results.is_empty() {
            messages.push(json!({"role": "user", "content": std::mem::take(results)}));
        }
    };
    for message in request.messages {
        match message {
            ChatMessage::ToolResult {
                call_id,
                content,
                is_error,
            } => results.push(json!({
                "type": "tool_result",
                "tool_use_id": call_id,
                "content": content,
                "is_error": is_error,
            })),
            ChatMessage::User(text) => {
                flush(&mut messages, &mut results);
                messages.push(json!({"role": "user", "content": text}));
            }
            ChatMessage::Assistant { text, tool_calls } => {
                flush(&mut messages, &mut results);
                let mut blocks = Vec::new();
                if !text.is_empty() {
                    blocks.push(json!({"type": "text", "text": text}));
                }
                for call in tool_calls {
                    blocks.push(json!({
                        "type": "tool_use",
                        "id": call.id,
                        "name": call.name,
                        "input": call.arguments,
                    }));
                }
                messages.push(json!({"role": "assistant", "content": blocks}));
            }
        }
    }
    flush(&mut messages, &mut results);
    let mut body = json!({
        "model": request.model,
        "max_tokens": request.max_output.unwrap_or(ANTHROPIC_STREAM_DEFAULT_MAX_TOKENS),
        "messages": messages,
        "stream": true,
    });
    if let Some(system) = request.system {
        body["system"] = json!(system);
    }
    if !request.tools.is_empty() {
        body["tools"] = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "name": tool.name,
                    "description": tool.description,
                    "input_schema": tool.input_schema,
                })
            })
            .collect();
    }
    body
}

enum Block {
    Text,
    Tool {
        id: String,
        name: String,
        input: String,
    },
    /// Thinking and anything unknown.
    Ignored,
}

#[derive(Default)]
pub(super) struct AnthropicDecoder {
    blocks: BTreeMap<u64, Block>,
    reason: Option<FinishReason>,
    usage: Usage,
    any_output: bool,
}

impl AnthropicDecoder {
    fn usage(&mut self, usage: Option<&Value>) {
        let Some(usage) = usage else { return };
        if let Some(n) = token_count(usage.get("input_tokens")) {
            self.usage.input_tokens = Some(n);
        }
        if let Some(n) = token_count(usage.get("output_tokens")) {
            self.usage.output_tokens = Some(n);
        }
    }

    fn finished(&mut self, out: &mut Vec<StreamEvent>) {
        let usage = &self.usage;
        let usage =
            (usage.input_tokens.is_some() || usage.output_tokens.is_some()).then(|| usage.clone());
        out.push(StreamEvent::Finished {
            reason: self.reason.take().unwrap_or(FinishReason::Other),
            usage,
        });
    }
}

fn index(value: &Value) -> u64 {
    value.get("index").and_then(Value::as_u64).unwrap_or(0)
}

impl Decoder for AnthropicDecoder {
    fn event(&mut self, event: &SseEvent, out: &mut Vec<StreamEvent>) -> Result<bool, ErrorCode> {
        let data = event.data.trim();
        if data.is_empty() {
            return Ok(false);
        }
        let value: Value = serde_json::from_str(data).map_err(|_| ErrorCode::AiInvalidResponse)?;
        let kind = value
            .get("type")
            .and_then(Value::as_str)
            .or(event.event.as_deref())
            .unwrap_or("");
        match kind {
            "message_start" => self.usage(value.get("message").and_then(|m| m.get("usage"))),
            "content_block_start" => {
                let block = value.get("content_block");
                let kind = block.and_then(|b| b.get("type")).and_then(Value::as_str);
                let text_of = |key: &str| {
                    block
                        .and_then(|b| b.get(key))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_owned()
                };
                let entry = match kind {
                    Some("text") => {
                        let text = text_of("text");
                        if !text.is_empty() {
                            self.any_output = true;
                            out.push(StreamEvent::Text(text));
                        }
                        Block::Text
                    }
                    Some("tool_use") => {
                        self.any_output = true;
                        Block::Tool {
                            id: text_of("id"),
                            name: text_of("name"),
                            input: String::new(),
                        }
                    }
                    _ => Block::Ignored,
                };
                self.blocks.insert(index(&value), entry);
            }
            "content_block_delta" => {
                let delta = value.get("delta");
                let kind = delta.and_then(|d| d.get("type")).and_then(Value::as_str);
                match (kind, self.blocks.get_mut(&index(&value))) {
                    (Some("text_delta"), Some(Block::Text)) => {
                        if let Some(text) = delta
                            .and_then(|d| d.get("text"))
                            .and_then(Value::as_str)
                            .filter(|t| !t.is_empty())
                        {
                            self.any_output = true;
                            out.push(StreamEvent::Text(text.to_owned()));
                        }
                    }
                    (Some("input_json_delta"), Some(Block::Tool { input, .. })) => {
                        if let Some(part) = delta
                            .and_then(|d| d.get("partial_json"))
                            .and_then(Value::as_str)
                        {
                            input.push_str(part);
                        }
                    }
                    _ => {}
                }
            }
            "content_block_stop" => {
                if let Some(Block::Tool { id, name, input }) = self.blocks.remove(&index(&value)) {
                    if name.is_empty() || id.is_empty() {
                        return Err(ErrorCode::AiInvalidResponse);
                    }
                    out.push(StreamEvent::ToolCall(ToolCallRequest {
                        id,
                        name,
                        arguments: parse_arguments(&input)?,
                    }));
                }
            }
            "message_delta" => {
                if let Some(reason) = value
                    .get("delta")
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(Value::as_str)
                {
                    self.reason = Some(match reason {
                        "end_turn" | "stop_sequence" => FinishReason::Stop,
                        "tool_use" => FinishReason::ToolCalls,
                        "max_tokens" => FinishReason::Length,
                        _ => FinishReason::Other,
                    });
                }
                self.usage(value.get("usage"));
            }
            "message_stop" => {
                self.finished(out);
                return Ok(true);
            }
            "error" => {
                let kind = value
                    .get("error")
                    .and_then(|e| e.get("type"))
                    .and_then(Value::as_str);
                return Err(match kind {
                    Some("rate_limit_error") => ErrorCode::AiRateLimited,
                    _ => ErrorCode::AiUnavailable,
                });
            }
            // `ping` and event types added later.
            _ => {}
        }
        Ok(false)
    }

    /// The body ended without `message_stop`: fine if a stop reason or any
    /// output arrived, an error if the answer is empty.
    fn end(&mut self, out: &mut Vec<StreamEvent>) -> Result<(), ErrorCode> {
        let cut_tool_call = self
            .blocks
            .values()
            .any(|b| matches!(b, Block::Tool { .. }));
        if cut_tool_call || (self.reason.is_none() && !self.any_output) {
            return Err(ErrorCode::AiInvalidResponse);
        }
        self.finished(out);
        Ok(())
    }
}
