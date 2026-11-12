//! The two wire protocols: listing models and one non-streaming completion.
//! Provider answers never leave this module as text: only fixed error codes
//! (by status) and parsed fields do, and keys are only ever header values.

use std::{
    fmt,
    time::{Duration, Instant},
};

use hyper::{
    Method,
    header::{self, HeaderName, HeaderValue},
};
use serde_json::{Value, json};
use url::Url;

use super::settings::AiProtocol;
use crate::{
    ErrorCode,
    net::{
        NetRoute,
        http::{self, HttpError, HttpReply, HttpRequest},
    },
};

pub(crate) const LIST_TIMEOUT: Duration = Duration::from_secs(15);
pub(crate) const CHAT_TIMEOUT: Duration = Duration::from_secs(60);
pub(crate) const MAX_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const ANTHROPIC_VERSION: &str = "2023-06-01";
const ANTHROPIC_DEFAULT_MAX_TOKENS: u32 = 1024;
const TEST_PROMPT: &str = "Reply with OK.";
/// Entries kept from one model list; a provider has no use for more.
const MAX_MODELS: usize = 5000;
const MAX_MODEL_ID_BYTES: usize = 512;

/// Where a request goes and what it signs with.
pub(crate) struct Endpoint<'a> {
    pub protocol: AiProtocol,
    /// Normalized: no trailing `/`, no query.
    pub base_url: &'a str,
    /// `None` or empty sends no credentials (local servers).
    pub key: Option<&'a str>,
    pub route: &'a NetRoute,
}

impl fmt::Debug for Endpoint<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Endpoint")
            .field("protocol", &self.protocol)
            .field("base_url", &self.base_url)
            .field("key", &self.key.map(|_| "[REDACTED]"))
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProviderModel {
    pub id: String,
    pub name: Option<String>,
}

pub(crate) fn http_error(error: HttpError) -> ErrorCode {
    match error {
        HttpError::Proxy => ErrorCode::AiProxy,
        HttpError::Timeout => ErrorCode::AiTimeout,
        HttpError::TooLarge => ErrorCode::AiInvalidResponse,
        HttpError::Network | HttpError::Invalid => ErrorCode::AiNetwork,
    }
}

/// The fixed code for a status that is not a success. `chat` is true for the
/// test call, where a 404 means the model does not exist.
pub(crate) fn status_error(status: u16, chat: bool) -> ErrorCode {
    match status {
        401 | 403 => ErrorCode::AiKeyRejected,
        402 => ErrorCode::AiQuotaExceeded,
        404 if chat => ErrorCode::AiModelNotFound,
        429 => ErrorCode::AiRateLimited,
        300..=499 => ErrorCode::AiRequestRejected,
        500..=599 => ErrorCode::AiUnavailable,
        _ => ErrorCode::AiInvalidResponse,
    }
}

fn auth_headers(endpoint: &Endpoint<'_>) -> Result<Vec<(HeaderName, HeaderValue)>, ErrorCode> {
    let mut headers = Vec::new();
    let key = endpoint.key.filter(|key| !key.is_empty());
    match endpoint.protocol {
        AiProtocol::OpenaiCompatible => {
            if let Some(key) = key {
                let value = http::sensitive_header(&format!("Bearer {key}"))
                    .ok_or(ErrorCode::AiKeyRejected)?;
                headers.push((header::AUTHORIZATION, value));
            }
        }
        AiProtocol::Anthropic => {
            if let Some(key) = key {
                let value = http::sensitive_header(key).ok_or(ErrorCode::AiKeyRejected)?;
                headers.push((HeaderName::from_static("x-api-key"), value));
            }
            headers.push((
                HeaderName::from_static("anthropic-version"),
                HeaderValue::from_static(ANTHROPIC_VERSION),
            ));
        }
    }
    Ok(headers)
}

fn url(endpoint: &Endpoint<'_>, suffix: &str) -> Result<Url, ErrorCode> {
    Url::parse(&format!("{}{suffix}", endpoint.base_url)).map_err(|_| ErrorCode::AiNetwork)
}

async fn request(
    endpoint: &Endpoint<'_>,
    method: Method,
    suffix: &str,
    body: Option<&Value>,
    timeout: Duration,
) -> Result<HttpReply, ErrorCode> {
    let url = url(endpoint, suffix)?;
    http::send(HttpRequest {
        method,
        url: &url,
        route: endpoint.route,
        headers: auth_headers(endpoint)?,
        body: body.map(|body| serde_json::to_vec(body).unwrap_or_default()),
        timeout,
        max_response_bytes: MAX_RESPONSE_BYTES,
    })
    .await
    .map_err(http_error)
}

/// `Ok(None)` when the provider has no model list (404 or 405): the caller
/// falls back to the catalog.
pub(crate) async fn list_models(
    endpoint: &Endpoint<'_>,
) -> Result<Option<Vec<ProviderModel>>, ErrorCode> {
    let suffix = match endpoint.protocol {
        AiProtocol::OpenaiCompatible => "/models",
        AiProtocol::Anthropic => "/models?limit=1000",
    };
    let reply = request(endpoint, Method::GET, suffix, None, LIST_TIMEOUT).await?;
    match reply.status {
        200 => parse_models(&reply.body).map(Some),
        404 | 405 => Ok(None),
        status => Err(status_error(status, false)),
    }
}

fn text_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

/// `{"data":[{"id", "name"? | "display_name"?}]}` for both protocols. Sorted
/// by id, one entry per id; entries without a usable id are skipped.
fn parse_models(body: &[u8]) -> Result<Vec<ProviderModel>, ErrorCode> {
    let value: Value = serde_json::from_slice(body).map_err(|_| ErrorCode::AiInvalidResponse)?;
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or(ErrorCode::AiInvalidResponse)?;
    let mut models: Vec<ProviderModel> = Vec::new();
    for entry in data {
        let Some(id) = text_field(entry, "id") else {
            continue;
        };
        if id.len() > MAX_MODEL_ID_BYTES || id.chars().any(char::is_control) {
            continue;
        }
        let name = text_field(entry, "display_name")
            .or_else(|| text_field(entry, "name"))
            .filter(|name| name != &id);
        models.push(ProviderModel { id, name });
    }
    models.sort_by(|a, b| a.id.cmp(&b.id));
    models.dedup_by(|later, first| later.id == first.id);
    models.truncate(MAX_MODELS);
    Ok(models)
}

/// One non-streaming request. Returns the answer once its top-level shape is
/// right (`choices` or `content` is an array).
async fn chat(
    endpoint: &Endpoint<'_>,
    model: &str,
    system: Option<&str>,
    user: &str,
    max_output: Option<u32>,
) -> Result<Value, ErrorCode> {
    let (suffix, body) = match endpoint.protocol {
        AiProtocol::OpenaiCompatible => {
            let mut messages = Vec::new();
            if let Some(system) = system {
                messages.push(json!({"role": "system", "content": system}));
            }
            messages.push(json!({"role": "user", "content": user}));
            let mut body = json!({"model": model, "messages": messages, "stream": false});
            if let Some(max) = max_output {
                body["max_tokens"] = json!(max);
            }
            ("/chat/completions", body)
        }
        AiProtocol::Anthropic => {
            let mut body = json!({
                "model": model,
                "max_tokens": max_output.unwrap_or(ANTHROPIC_DEFAULT_MAX_TOKENS),
                "messages": [{"role": "user", "content": user}],
            });
            if let Some(system) = system {
                body["system"] = json!(system);
            }
            ("/messages", body)
        }
    };
    let reply = request(endpoint, Method::POST, suffix, Some(&body), CHAT_TIMEOUT).await?;
    if reply.status != 200 {
        return Err(status_error(reply.status, true));
    }
    let value: Value =
        serde_json::from_slice(&reply.body).map_err(|_| ErrorCode::AiInvalidResponse)?;
    let shape = match endpoint.protocol {
        AiProtocol::OpenaiCompatible => "choices",
        AiProtocol::Anthropic => "content",
    };
    if value.get(shape).is_some_and(Value::is_array) {
        Ok(value)
    } else {
        Err(ErrorCode::AiInvalidResponse)
    }
}

fn text_parts(parts: &[Value]) -> String {
    parts
        .iter()
        .filter(|part| {
            part.get("type")
                .and_then(Value::as_str)
                .is_none_or(|t| t == "text")
        })
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect()
}

/// The reply text of a checked answer.
fn completion_text(protocol: AiProtocol, value: &Value) -> Option<String> {
    match protocol {
        AiProtocol::OpenaiCompatible => {
            let content = value
                .get("choices")?
                .get(0)?
                .get("message")?
                .get("content")?;
            match content {
                Value::String(text) => Some(text.clone()),
                Value::Array(parts) => Some(text_parts(parts)),
                _ => None,
            }
        }
        AiProtocol::Anthropic => Some(text_parts(value.get("content")?.as_array()?)),
    }
}

/// The non-streaming completion primitive: one system prompt (optional), one
/// user message, the reply text. `max_output` caps the reply; OpenAI-compatible
/// servers get no cap when it is `None` (newer OpenAI models reject
/// `max_tokens`), Anthropic needs one and defaults to 1024.
pub(crate) async fn complete(
    endpoint: &Endpoint<'_>,
    model: &str,
    system: Option<&str>,
    user: &str,
    max_output: Option<u32>,
) -> Result<String, ErrorCode> {
    let value = chat(endpoint, model, system, user, max_output).await?;
    completion_text(endpoint.protocol, &value).ok_or(ErrorCode::AiInvalidResponse)
}

/// A fixed English prompt (no mail text). Success is a 200 answer of the
/// expected shape; returns the time it took in milliseconds.
pub(crate) async fn test(endpoint: &Endpoint<'_>, model: &str) -> Result<u32, ErrorCode> {
    let max_output = (endpoint.protocol == AiProtocol::Anthropic).then_some(16);
    let started = Instant::now();
    chat(endpoint, model, None, TEST_PROMPT, max_output).await?;
    Ok(u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX))
}

#[cfg(test)]
pub(crate) mod tests;
