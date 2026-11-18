//! Anthropic Messages: request body and stream decoding.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::{
    ANTHROPIC_STREAM_DEFAULT_MAX_TOKENS, ChatMessage, ChatRequest, Decoder, FinishReason,
    StreamEvent, ToolCallRequest, Usage, parse_arguments, sse::SseEvent, token_count,
};
use crate::{
    ErrorCode,
    ai::reasoning::{AnthropicPart, ReasoningEcho, echo_for},
};

fn text_block(text: &str) -> Value {
    json!({"type": "text", "text": text})
}

fn tool_block(call: &ToolCallRequest) -> Value {
    json!({
        "type": "tool_use",
        "id": call.id,
        "name": call.name,
        "input": call.arguments,
    })
}

fn thinking_block(part: &AnthropicPart) -> Option<Value> {
    match part {
        AnthropicPart::Thinking {
            thinking,
            signature,
        } => {
            let mut block = json!({"type": "thinking", "thinking": thinking});
            if let Some(signature) = signature {
                block["signature"] = json!(signature);
            }
            Some(block)
        }
        AnthropicPart::RedactedThinking { data } => {
            Some(json!({"type": "redacted_thinking", "data": data}))
        }
        AnthropicPart::Text { .. } | AnthropicPart::ToolUse { .. } => None,
    }
}

/// The content of an assistant message. With reasoning from this server the
/// blocks follow the order they arrived in, thinking blocks unchanged;
/// anything the recorded order does not account for goes after it.
fn assistant_blocks(
    text: &str,
    tool_calls: &[ToolCallRequest],
    parts: Option<&[AnthropicPart]>,
) -> Vec<Value> {
    let mut blocks = Vec::new();
    let mut at = 0usize;
    let mut used = vec![false; tool_calls.len()];
    for part in parts.unwrap_or_default() {
        match part {
            AnthropicPart::Text { bytes } => {
                let end = at.saturating_add(*bytes);
                match text.get(at..end) {
                    Some(slice) if !slice.is_empty() => {
                        blocks.push(text_block(slice));
                        at = end;
                    }
                    _ => {}
                }
            }
            AnthropicPart::ToolUse { id } => {
                if let Some(index) = tool_calls
                    .iter()
                    .enumerate()
                    .position(|(i, call)| !used[i] && call.id == *id)
                {
                    used[index] = true;
                    blocks.push(tool_block(&tool_calls[index]));
                }
            }
            thinking => blocks.extend(thinking_block(thinking)),
        }
    }
    if let Some(rest) = text.get(at..).filter(|rest| !rest.is_empty()) {
        blocks.push(text_block(rest));
    }
    for (call, _) in tool_calls.iter().zip(&used).filter(|(_, used)| !**used) {
        blocks.push(tool_block(call));
    }
    blocks
}

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
            ChatMessage::Assistant {
                text,
                tool_calls,
                reasoning,
            } => {
                flush(&mut messages, &mut results);
                let parts = match echo_for(reasoning.as_ref(), request.origin) {
                    Some(ReasoningEcho::Blocks { parts }) => Some(parts.as_slice()),
                    _ => None,
                };
                let blocks = assistant_blocks(text, tool_calls, parts);
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
    /// `bytes` of text so far.
    Text {
        bytes: usize,
    },
    Tool {
        id: String,
        name: String,
        input: String,
    },
    Thinking {
        thinking: String,
        signature: Option<String>,
    },
    Redacted {
        data: String,
    },
    /// Anything unknown.
    Ignored,
}

#[derive(Default)]
pub(super) struct AnthropicDecoder {
    blocks: BTreeMap<u64, Block>,
    reason: Option<FinishReason>,
    usage: Usage,
    any_output: bool,
    /// Finished blocks in order, kept for sending reasoning back.
    parts: Vec<AnthropicPart>,
    any_thinking: bool,
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
        // Blocks the stream never closed still count, in index order.
        for (_, block) in std::mem::take(&mut self.blocks) {
            self.close(block);
        }
        if self.any_thinking {
            out.push(StreamEvent::ReasoningEcho(ReasoningEcho::Blocks {
                parts: std::mem::take(&mut self.parts),
            }));
        }
        let usage = &self.usage;
        let usage =
            (usage.input_tokens.is_some() || usage.output_tokens.is_some()).then(|| usage.clone());
        out.push(StreamEvent::Finished {
            reason: self.reason.take().unwrap_or(FinishReason::Other),
            usage,
        });
    }
}

impl AnthropicDecoder {
    /// Records a finished block's place (and a thinking block's content).
    fn close(&mut self, block: Block) {
        let part = match block {
            Block::Text { bytes } if bytes > 0 => AnthropicPart::Text { bytes },
            Block::Tool { id, .. } => AnthropicPart::ToolUse { id },
            Block::Thinking {
                thinking,
                signature,
            } => AnthropicPart::Thinking {
                thinking,
                signature,
            },
            Block::Redacted { data } => AnthropicPart::RedactedThinking { data },
            Block::Text { .. } | Block::Ignored => return,
        };
        self.any_thinking |= matches!(
            part,
            AnthropicPart::Thinking { .. } | AnthropicPart::RedactedThinking { .. }
        );
        self.parts.push(part);
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
                        let bytes = text.len();
                        if !text.is_empty() {
                            self.any_output = true;
                            out.push(StreamEvent::Text(text));
                        }
                        Block::Text { bytes }
                    }
                    Some("thinking") => {
                        let thinking = text_of("thinking");
                        out.push(StreamEvent::Reasoning(thinking.clone()));
                        let signature = block
                            .and_then(|b| b.get("signature"))
                            .and_then(Value::as_str)
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned);
                        Block::Thinking {
                            thinking,
                            signature,
                        }
                    }
                    Some("redacted_thinking") => {
                        out.push(StreamEvent::Reasoning(String::new()));
                        Block::Redacted {
                            data: text_of("data"),
                        }
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
                    (Some("text_delta"), Some(Block::Text { bytes })) => {
                        if let Some(text) = delta
                            .and_then(|d| d.get("text"))
                            .and_then(Value::as_str)
                            .filter(|t| !t.is_empty())
                        {
                            *bytes += text.len();
                            self.any_output = true;
                            out.push(StreamEvent::Text(text.to_owned()));
                        }
                    }
                    (Some("thinking_delta"), Some(Block::Thinking { thinking, .. })) => {
                        if let Some(text) = delta
                            .and_then(|d| d.get("thinking"))
                            .and_then(Value::as_str)
                            .filter(|t| !t.is_empty())
                        {
                            thinking.push_str(text);
                            out.push(StreamEvent::Reasoning(text.to_owned()));
                        }
                    }
                    (Some("signature_delta"), Some(Block::Thinking { signature, .. })) => {
                        if let Some(part) = delta
                            .and_then(|d| d.get("signature"))
                            .and_then(Value::as_str)
                        {
                            signature.get_or_insert_with(String::new).push_str(part);
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
            "content_block_stop" => match self.blocks.remove(&index(&value)) {
                Some(Block::Tool { id, name, input }) => {
                    if name.is_empty() || id.is_empty() {
                        return Err(ErrorCode::AiInvalidResponse);
                    }
                    out.push(StreamEvent::ToolCall(ToolCallRequest {
                        id: id.clone(),
                        name,
                        arguments: parse_arguments(&input)?,
                    }));
                    self.parts.push(AnthropicPart::ToolUse { id });
                }
                Some(block) => self.close(block),
                None => {}
            },
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
