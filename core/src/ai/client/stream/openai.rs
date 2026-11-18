//! OpenAI-compatible Chat Completions: request body and stream decoding.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::{
    ChatMessage, ChatRequest, Decoder, FinishReason, StreamEvent, ToolCallRequest, Usage,
    parse_arguments, sse::SseEvent, token_count,
};
use crate::{
    ErrorCode,
    ai::reasoning::{ReasoningEcho, ReasoningField, echo_for},
};

pub(super) fn body(request: &ChatRequest<'_>) -> Value {
    let mut messages = Vec::new();
    if let Some(system) = request.system {
        messages.push(json!({"role": "system", "content": system}));
    }
    for message in request.messages {
        messages.push(match message {
            ChatMessage::User(text) => json!({"role": "user", "content": text}),
            ChatMessage::Assistant {
                text,
                tool_calls,
                reasoning,
            } => {
                let content = if text.is_empty() && !tool_calls.is_empty() {
                    Value::Null
                } else {
                    json!(text)
                };
                let mut message = json!({"role": "assistant", "content": content});
                // Only what this server sent, under the name it used.
                if let Some(ReasoningEcho::Field { field, text }) =
                    echo_for(reasoning.as_ref(), request.origin)
                {
                    message[field.name()] = json!(text);
                }
                if !tool_calls.is_empty() {
                    message["tool_calls"] = tool_calls
                        .iter()
                        .map(|call| {
                            json!({
                                "id": call.id,
                                "type": "function",
                                "function": {
                                    "name": call.name,
                                    "arguments": call.arguments.to_string(),
                                },
                            })
                        })
                        .collect();
                }
                message
            }
            ChatMessage::ToolResult {
                call_id, content, ..
            } => json!({"role": "tool", "tool_call_id": call_id, "content": content}),
        });
    }
    let mut body = json!({"model": request.model, "messages": messages, "stream": true});
    if !request.tools.is_empty() {
        body["tools"] = request
            .tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.input_schema,
                    },
                })
            })
            .collect();
    }
    if let Some(max) = request.max_output {
        body["max_tokens"] = json!(max);
    }
    body
}

#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Default)]
pub(super) struct OpenAiDecoder {
    calls: BTreeMap<u64, PartialCall>,
    reason: Option<FinishReason>,
    usage: Option<Usage>,
    any_output: bool,
    last_index: Option<u64>,
    /// The field the server's reasoning came in, once one did.
    reasoning_field: Option<ReasoningField>,
    reasoning: String,
}

impl OpenAiDecoder {
    fn tool_fragment(&mut self, fragment: &Value) {
        let id = fragment.get("id").and_then(Value::as_str).unwrap_or("");
        // Servers that leave out `index` start a new call with each new id.
        let index = match fragment.get("index").and_then(Value::as_u64) {
            Some(index) => index,
            None if !id.is_empty() || self.last_index.is_none() => {
                self.calls.keys().next_back().map_or(0, |last| last + 1)
            }
            None => self.last_index.unwrap_or(0),
        };
        self.last_index = Some(index);
        let call = self.calls.entry(index).or_default();
        if call.id.is_empty() {
            call.id = id.to_owned();
        }
        let function = fragment.get("function");
        if let Some(name) = function.and_then(|f| f.get("name")).and_then(Value::as_str) {
            // Some servers repeat the whole name in every fragment.
            if call.name.is_empty() {
                call.name = name.to_owned();
            }
        }
        if let Some(arguments) = function
            .and_then(|f| f.get("arguments"))
            .and_then(Value::as_str)
        {
            call.arguments.push_str(arguments);
        }
        self.any_output = true;
    }

    /// Reasoning text of a delta: `reasoning_content`, or `reasoning` when it
    /// is a string. The first field seen is the one kept.
    fn reasoning_fragment(&mut self, delta: &Value, out: &mut Vec<StreamEvent>) {
        let fields = match self.reasoning_field {
            Some(field) => [Some(field), None],
            None => [
                Some(ReasoningField::ReasoningContent),
                Some(ReasoningField::Reasoning),
            ],
        };
        for field in fields.into_iter().flatten() {
            if let Some(text) = delta
                .get(field.name())
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty())
            {
                self.reasoning_field = Some(field);
                self.reasoning.push_str(text);
                out.push(StreamEvent::Reasoning(text.to_owned()));
                return;
            }
        }
    }

    fn finish(&mut self, out: &mut Vec<StreamEvent>) -> Result<(), ErrorCode> {
        let mut emitted = false;
        for (index, call) in std::mem::take(&mut self.calls) {
            if call.name.is_empty() {
                return Err(ErrorCode::AiInvalidResponse);
            }
            let id = if call.id.is_empty() {
                format!("call_{index}")
            } else {
                call.id
            };
            out.push(StreamEvent::ToolCall(ToolCallRequest {
                id,
                name: call.name,
                arguments: parse_arguments(&call.arguments)?,
            }));
            emitted = true;
        }
        if let Some(field) = self.reasoning_field.take() {
            out.push(StreamEvent::ReasoningEcho(ReasoningEcho::Field {
                field,
                text: std::mem::take(&mut self.reasoning),
            }));
        }
        let reason = match self.reason.take() {
            // Some servers report `stop` after tool calls.
            Some(FinishReason::Stop) if emitted => FinishReason::ToolCalls,
            Some(reason) => reason,
            None => FinishReason::Other,
        };
        out.push(StreamEvent::Finished {
            reason,
            usage: self.usage.take(),
        });
        Ok(())
    }
}

fn error_code(value: &Value) -> ErrorCode {
    let kind = value
        .get("error")
        .and_then(|e| e.get("type").or_else(|| e.get("code")))
        .and_then(Value::as_str)
        .unwrap_or("");
    if kind.contains("rate_limit") {
        ErrorCode::AiRateLimited
    } else {
        ErrorCode::AiUnavailable
    }
}

impl Decoder for OpenAiDecoder {
    fn event(&mut self, event: &SseEvent, out: &mut Vec<StreamEvent>) -> Result<bool, ErrorCode> {
        let data = event.data.trim();
        if data == "[DONE]" {
            self.end(out)?;
            return Ok(true);
        }
        if data.is_empty() {
            return Ok(false);
        }
        let value: Value = serde_json::from_str(data).map_err(|_| ErrorCode::AiInvalidResponse)?;
        if value.get("error").is_some_and(|e| !e.is_null()) {
            return Err(error_code(&value));
        }
        if let Some(usage) = value.get("usage").filter(|u| u.is_object()) {
            self.usage = Some(Usage {
                input_tokens: token_count(usage.get("prompt_tokens")),
                output_tokens: token_count(usage.get("completion_tokens")),
            });
        }
        let Some(choice) = value.get("choices").and_then(|c| c.get(0)) else {
            return Ok(false);
        };
        if let Some(delta) = choice.get("delta") {
            self.reasoning_fragment(delta, out);
            if let Some(text) = delta
                .get("content")
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty())
            {
                self.any_output = true;
                out.push(StreamEvent::Text(text.to_owned()));
            }
            if let Some(fragments) = delta.get("tool_calls").and_then(Value::as_array) {
                for fragment in fragments {
                    self.tool_fragment(fragment);
                }
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
            self.reason = Some(match reason {
                "stop" => FinishReason::Stop,
                "tool_calls" | "function_call" => FinishReason::ToolCalls,
                "length" => FinishReason::Length,
                _ => FinishReason::Other,
            });
        }
        Ok(false)
    }

    /// The body ended without `[DONE]`: fine if a finish reason or any output
    /// arrived, an error if the answer is empty.
    fn end(&mut self, out: &mut Vec<StreamEvent>) -> Result<(), ErrorCode> {
        if self.reason.is_none() && !self.any_output {
            return Err(ErrorCode::AiInvalidResponse);
        }
        self.finish(out)
    }
}
