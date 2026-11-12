//! The provider catalog: a built-in table (works offline, addresses come from
//! here) plus providers and model facts trimmed from models.dev.
//!
//! models.dev is fetched on demand through the AI route, with the stored ETag,
//! trimmed to what MailX uses and cached as one JSON file. A failed fetch
//! keeps the old cache. Models are kept internally to label model lists; the
//! IPC catalog carries providers only.

use std::{collections::HashMap, fs, io::Write, path::Path};

use hyper::{
    Method,
    header::{self, HeaderValue},
};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use url::Url;
use uuid::Uuid;

use super::{
    client::http_error,
    settings::AiProtocol,
    validation::{
        CUSTOM_PROVIDER_ID, clean_model_name, normalize_base_url, valid_model_id, valid_provider_id,
    },
};
use crate::{
    ErrorCode,
    net::{
        NetRoute,
        http::{self, HttpRequest},
    },
};

pub(crate) const CATALOG_URL: &str = "https://models.dev/api.json";
pub(crate) const CATALOG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
pub(crate) const MAX_CATALOG_BYTES: usize = 16 * 1024 * 1024;
/// A cache older than this is reported as stale.
pub(crate) const STALE_AFTER_SECONDS: i64 = 24 * 60 * 60;
const CACHE_VERSION: u32 = 1;
const MAX_MODELS_PER_PROVIDER: usize = 3000;
const MAX_PROVIDER_NAME_CHARS: usize = 64;
const MAX_ETAG_BYTES: usize = 256;

pub(crate) struct BuiltIn {
    pub id: &'static str,
    pub name: &'static str,
    pub protocol: AiProtocol,
    pub base_url: &'static str,
    /// Runs on this computer: no key, and plain http is expected.
    pub local: bool,
}

const fn built_in(
    id: &'static str,
    name: &'static str,
    protocol: AiProtocol,
    base_url: &'static str,
    local: bool,
) -> BuiltIn {
    BuiltIn {
        id,
        name,
        protocol,
        base_url,
        local,
    }
}

/// The common providers, in display order. Ids and addresses follow
/// models.dev; where it lists a provider for one protocol only, that is the
/// one used (MiniMax lists only its Anthropic-style endpoint).
pub(crate) const BUILT_INS: &[BuiltIn] = &[
    built_in(
        "openai",
        "OpenAI",
        AiProtocol::OpenaiCompatible,
        "https://api.openai.com/v1",
        false,
    ),
    built_in(
        "anthropic",
        "Anthropic",
        AiProtocol::Anthropic,
        "https://api.anthropic.com/v1",
        false,
    ),
    built_in(
        "deepseek",
        "DeepSeek",
        AiProtocol::OpenaiCompatible,
        "https://api.deepseek.com",
        false,
    ),
    built_in(
        "openrouter",
        "OpenRouter",
        AiProtocol::OpenaiCompatible,
        "https://openrouter.ai/api/v1",
        false,
    ),
    built_in(
        "zhipuai",
        "Zhipu AI",
        AiProtocol::OpenaiCompatible,
        "https://open.bigmodel.cn/api/paas/v4",
        false,
    ),
    built_in(
        "alibaba-cn",
        "Alibaba (China)",
        AiProtocol::OpenaiCompatible,
        "https://dashscope.aliyuncs.com/compatible-mode/v1",
        false,
    ),
    built_in(
        "moonshotai-cn",
        "Moonshot AI (China)",
        AiProtocol::OpenaiCompatible,
        "https://api.moonshot.cn/v1",
        false,
    ),
    built_in(
        "siliconflow-cn",
        "SiliconFlow (China)",
        AiProtocol::OpenaiCompatible,
        "https://api.siliconflow.cn/v1",
        false,
    ),
    built_in(
        "volcengine",
        "Volcengine Ark",
        AiProtocol::OpenaiCompatible,
        "https://ark.cn-beijing.volces.com/api/v3",
        false,
    ),
    built_in(
        "minimax-cn",
        "MiniMax (China)",
        AiProtocol::Anthropic,
        "https://api.minimax.cn/anthropic/v1",
        false,
    ),
    built_in(
        "ollama",
        "Ollama",
        AiProtocol::OpenaiCompatible,
        "http://127.0.0.1:11434/v1",
        true,
    ),
    built_in(
        "lmstudio",
        "LM Studio",
        AiProtocol::OpenaiCompatible,
        "http://127.0.0.1:1234/v1",
        true,
    ),
];

fn built_in_by_id(id: &str) -> Option<&'static BuiltIn> {
    BUILT_INS.iter().find(|entry| entry.id == id)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AiCatalogProvider {
    pub id: String,
    pub name: String,
    pub protocol: AiProtocol,
    pub base_url: String,
    pub built_in: bool,
    /// Runs on this computer; no key needed.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub local: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AiCatalog {
    /// Built-in providers first, then the rest by name. The frontend adds
    /// "custom" itself.
    pub providers: Vec<AiCatalogProvider>,
    /// When the models.dev data was last confirmed current (RFC 3339);
    /// `None` without a cache.
    pub updated_at: Option<String>,
    /// No cache, or one older than 24 hours.
    pub stale: bool,
    /// A refresh was asked for and failed; the cache above is still in use.
    pub refresh_error: Option<ErrorCode>,
}

// ----------------------------------------------------------- cached data

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CatalogModel {
    pub id: String,
    pub name: String,
    pub context: Option<u32>,
    pub output: Option<u32>,
    pub tool_call: Option<bool>,
    /// False for models that do not output text (embedding, image, speech).
    /// They are only kept by id, to be left out of lists.
    pub text: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CatalogProvider {
    pub id: String,
    pub name: String,
    pub protocol: AiProtocol,
    pub base_url: String,
    pub models: Vec<CatalogModel>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CatalogData {
    version: u32,
    pub etag: Option<String>,
    /// Unix seconds of the last fetch that returned data or 304.
    pub fetched_at: i64,
    pub providers: Vec<CatalogProvider>,
}

impl CatalogData {
    pub(crate) fn new(
        etag: Option<String>,
        fetched_at: i64,
        providers: Vec<CatalogProvider>,
    ) -> Self {
        Self {
            version: CACHE_VERSION,
            etag,
            fetched_at,
            providers,
        }
    }

    pub(crate) fn provider(&self, id: &str) -> Option<&CatalogProvider> {
        self.providers.iter().find(|provider| provider.id == id)
    }

    pub(crate) fn stale(&self, now: i64) -> bool {
        now.saturating_sub(self.fetched_at) > STALE_AFTER_SECONDS
    }
}

/// The providers to show: the built-ins in table order, then the others by name.
pub(crate) fn providers(data: Option<&CatalogData>) -> Vec<AiCatalogProvider> {
    let mut out: Vec<AiCatalogProvider> = BUILT_INS
        .iter()
        .map(|entry| AiCatalogProvider {
            id: entry.id.to_owned(),
            name: entry.name.to_owned(),
            protocol: entry.protocol,
            base_url: entry.base_url.to_owned(),
            built_in: true,
            local: entry.local.then_some(true),
        })
        .collect();
    let mut rest: Vec<AiCatalogProvider> = data
        .into_iter()
        .flat_map(|data| &data.providers)
        .filter(|provider| built_in_by_id(&provider.id).is_none())
        .map(|provider| AiCatalogProvider {
            id: provider.id.clone(),
            name: provider.name.clone(),
            protocol: provider.protocol,
            base_url: provider.base_url.clone(),
            built_in: false,
            local: None,
        })
        .collect();
    rest.sort_by_cached_key(|provider| (provider.name.to_lowercase(), provider.id.clone()));
    out.extend(rest);
    out
}

// ------------------------------------------------------------------ trim

fn lenient<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(T::deserialize(deserializer).ok())
}

/// A value of the wrong type reads as absent instead of failing the file.
struct Lenient<T>(Option<T>);

impl<'de, T: serde::de::DeserializeOwned> Deserialize<'de> for Lenient<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        // Buffer first so a failed `T` still consumes the value.
        let value = serde_json::Value::deserialize(deserializer)?;
        Ok(Self(serde_json::from_value(value).ok()))
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawLimit {
    #[serde(deserialize_with = "lenient")]
    context: Option<u64>,
    #[serde(deserialize_with = "lenient")]
    output: Option<u64>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawModalities {
    #[serde(deserialize_with = "lenient")]
    output: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawModel {
    #[serde(deserialize_with = "lenient")]
    id: Option<String>,
    #[serde(deserialize_with = "lenient")]
    name: Option<String>,
    #[serde(deserialize_with = "lenient")]
    tool_call: Option<bool>,
    #[serde(deserialize_with = "lenient", rename = "type")]
    kind: Option<String>,
    #[serde(deserialize_with = "lenient")]
    limit: Option<RawLimit>,
    #[serde(deserialize_with = "lenient")]
    modalities: Option<RawModalities>,
    /// Present when the model is served by a different provider setup.
    provider: Option<serde::de::IgnoredAny>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawProvider {
    #[serde(deserialize_with = "lenient")]
    id: Option<String>,
    #[serde(deserialize_with = "lenient")]
    name: Option<String>,
    #[serde(deserialize_with = "lenient")]
    npm: Option<String>,
    #[serde(deserialize_with = "lenient")]
    api: Option<String>,
    #[serde(deserialize_with = "lenient")]
    env: Option<Vec<String>>,
    #[serde(deserialize_with = "lenient")]
    models: Option<HashMap<String, Lenient<RawModel>>>,
}

fn protocol_for_npm(npm: &str) -> Option<AiProtocol> {
    match npm {
        "@ai-sdk/openai-compatible" | "@ai-sdk/openai" => Some(AiProtocol::OpenaiCompatible),
        "@ai-sdk/anthropic" => Some(AiProtocol::Anthropic),
        _ => None,
    }
}

/// How a provider connects: the built-in table wins; otherwise the npm package
/// picks the protocol and the entry needs a plain address and at most one
/// credential. `None` means the provider is not offered.
fn route_for(id: &str, raw: &RawProvider) -> Option<(AiProtocol, String)> {
    if let Some(entry) = built_in_by_id(id) {
        return Some((entry.protocol, entry.base_url.to_owned()));
    }
    let protocol = protocol_for_npm(raw.npm.as_deref()?)?;
    if raw.env.as_ref().is_some_and(|env| env.len() > 1) {
        return None;
    }
    let api = raw.api.as_deref()?;
    if api.contains("${") {
        return None;
    }
    Some((protocol, normalize_base_url(api)?))
}

fn trim_model(raw: RawModel) -> Option<CatalogModel> {
    if raw.provider.is_some() {
        return None;
    }
    let id = raw.id.filter(|id| valid_model_id(id))?;
    let limit = raw.limit.unwrap_or_default();
    let number = |value: Option<u64>| value.and_then(|v| u32::try_from(v).ok()).filter(|v| *v > 0);
    let text = raw
        .modalities
        .and_then(|m| m.output)
        .is_none_or(|out| out.iter().any(|kind| kind == "text"))
        && raw.kind.as_deref().is_none_or(|kind| kind == "chat");
    if !text {
        return Some(CatalogModel {
            name: id.clone(),
            id,
            context: None,
            output: None,
            tool_call: None,
            text: false,
        });
    }
    Some(CatalogModel {
        name: raw
            .name
            .as_deref()
            .and_then(clean_model_name)
            .unwrap_or_else(|| id.clone()),
        id,
        context: number(limit.context),
        output: number(limit.output),
        tool_call: raw.tool_call,
        text: true,
    })
}

/// Reduces the models.dev document to the usable providers (spec §6). `None`
/// when the document is not a provider map at all.
pub(crate) fn trim(bytes: &[u8]) -> Option<Vec<CatalogProvider>> {
    let raw: HashMap<String, Lenient<RawProvider>> = serde_json::from_slice(bytes).ok()?;
    let mut providers = Vec::new();
    for (key, raw) in raw {
        let Some(raw) = raw.0 else { continue };
        let id = raw.id.clone().unwrap_or(key);
        if !valid_provider_id(&id) || id == CUSTOM_PROVIDER_ID {
            continue;
        }
        let Some((protocol, base_url)) = route_for(&id, &raw) else {
            continue;
        };
        let name = raw
            .name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty() && !name.chars().any(char::is_control))
            .map_or_else(
                || id.clone(),
                |name| name.chars().take(MAX_PROVIDER_NAME_CHARS).collect(),
            );
        let mut models: Vec<CatalogModel> = raw
            .models
            .unwrap_or_default()
            .into_values()
            .filter_map(|model| trim_model(model.0?))
            .collect();
        models.sort_by(|a, b| a.id.cmp(&b.id));
        models.dedup_by(|later, first| later.id == first.id);
        models.truncate(MAX_MODELS_PER_PROVIDER);
        providers.push(CatalogProvider {
            id,
            name,
            protocol,
            base_url,
            models,
        });
    }
    providers.sort_by(|a, b| a.id.cmp(&b.id));
    Some(providers)
}

// ------------------------------------------------------------------ disk

pub(crate) const CACHE_FILE: &str = "catalog.json";
const MAX_CACHE_FILE_BYTES: u64 = 32 * 1024 * 1024;

/// `None` for a missing, damaged or newer-version cache: all read as "no cache".
pub(crate) fn read_cache(directory: &Path) -> Option<CatalogData> {
    let path = directory.join(CACHE_FILE);
    let metadata = fs::symlink_metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_CACHE_FILE_BYTES {
        return None;
    }
    let data: CatalogData = serde_json::from_slice(&fs::read(&path).ok()?).ok()?;
    (data.version == CACHE_VERSION).then_some(data)
}

pub(crate) fn write_cache(directory: &Path, data: &CatalogData) -> std::io::Result<()> {
    fs::create_dir_all(directory)?;
    let temporary = directory.join(format!(".{CACHE_FILE}.{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&serde_json::to_vec(data).map_err(std::io::Error::other)?)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, directory.join(CACHE_FILE))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

// --------------------------------------------------------------- network

#[derive(Debug)]
pub(crate) enum Fetched {
    NotModified,
    Fresh {
        etag: Option<String>,
        providers: Vec<CatalogProvider>,
    },
}

/// A conditional GET of the catalog. Trimming a large document is CPU work, so
/// it runs off the async threads.
pub(crate) async fn fetch(
    route: &NetRoute,
    url: &Url,
    etag: Option<&str>,
) -> Result<Fetched, ErrorCode> {
    let mut headers = Vec::new();
    if let Some(value) = etag.and_then(|etag| HeaderValue::from_str(etag).ok()) {
        headers.push((header::IF_NONE_MATCH, value));
    }
    let reply = http::send(HttpRequest {
        method: Method::GET,
        url,
        route,
        headers,
        body: None,
        timeout: CATALOG_TIMEOUT,
        max_response_bytes: MAX_CATALOG_BYTES,
    })
    .await
    .map_err(http_error)?;
    match reply.status {
        304 => Ok(Fetched::NotModified),
        200 => {
            let etag = reply
                .headers
                .get(header::ETAG)
                .and_then(|value| value.to_str().ok())
                .filter(|value| value.len() <= MAX_ETAG_BYTES)
                .map(str::to_owned);
            let providers = tokio::task::spawn_blocking(move || trim(&reply.body))
                .await
                .ok()
                .flatten()
                .ok_or(ErrorCode::AiInvalidResponse)?;
            Ok(Fetched::Fresh { etag, providers })
        }
        status => Err(super::client::status_error(status, false)),
    }
}

#[cfg(test)]
pub(crate) mod tests;
