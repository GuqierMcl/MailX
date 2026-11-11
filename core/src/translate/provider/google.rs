//! Cloud Translation Basic (v2) with an API key, checked against
//! https://docs.cloud.google.com/translate/docs/reference/rest/v2/translate and
//! https://docs.cloud.google.com/docs/authentication/api-keys-use (key in the
//! `X-goog-api-key` header). The error shape of a rejected key is not in the
//! published pages (UNVERIFIED): it follows what the service is known to
//! return, `400` with reason `API_KEY_INVALID`. Only reason codes are read,
//! never the message text.
use hyper::header::HeaderName;
use serde::Deserialize;
use serde_json::json;

use super::{
    Context, TranslateRequest, TranslateResponse,
    http::{self, Base, Post},
    normalize_detected, require_target,
};
use crate::ErrorCode;

/// Basic uses NMT codes: Chinese by region, Norwegian as `no`.
pub(super) fn target_code(code: &str) -> Option<&'static str> {
    Some(match code {
        "zh-Hans" => "zh-CN",
        "zh-Hant" => "zh-TW",
        "nb" => "no",
        "en" => "en",
        "ja" => "ja",
        "ko" => "ko",
        "fr" => "fr",
        "de" => "de",
        "es" => "es",
        "pt" => "pt",
        "it" => "it",
        "ru" => "ru",
        "nl" => "nl",
        "pl" => "pl",
        "tr" => "tr",
        "ar" => "ar",
        "id" => "id",
        "uk" => "uk",
        "sv" => "sv",
        "da" => "da",
        "fi" => "fi",
        "cs" => "cs",
        "el" => "el",
        "hu" => "hu",
        "ro" => "ro",
        "bg" => "bg",
        _ => return None,
    })
}

#[derive(Deserialize)]
struct Answer {
    data: Data,
}

#[derive(Deserialize)]
struct Data {
    translations: Vec<Item>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Item {
    translated_text: String,
    detected_source_language: Option<String>,
}

#[derive(Deserialize, Default)]
struct Failure {
    #[serde(default)]
    error: FailureBody,
}

#[derive(Deserialize, Default)]
struct FailureBody {
    #[serde(default)]
    errors: Vec<Reason>,
    #[serde(default)]
    details: Vec<Reason>,
}

#[derive(Deserialize)]
struct Reason {
    reason: Option<String>,
}

impl Failure {
    fn has_reason(&self, wanted: &[&str]) -> bool {
        self.error
            .errors
            .iter()
            .chain(&self.error.details)
            .filter_map(|reason| reason.reason.as_deref())
            .any(|reason| wanted.contains(&reason))
    }
}

fn failure(status: u16, body: &[u8]) -> ErrorCode {
    let parsed: Failure = serde_json::from_slice(body).unwrap_or_default();
    match status {
        400 if parsed.has_reason(&["API_KEY_INVALID", "keyInvalid"]) => {
            ErrorCode::TranslationKeyRejected
        }
        403 if parsed.has_reason(&["dailyLimitExceeded", "quotaExceeded"]) => {
            ErrorCode::TranslationQuotaExceeded
        }
        403 if parsed.has_reason(&["userRateLimitExceeded", "rateLimitExceeded"]) => {
            ErrorCode::TranslationRateLimited
        }
        _ => http::common_status_error(status),
    }
}

pub(super) async fn translate(
    context: &Context<'_>,
    request: &TranslateRequest<'_>,
) -> Result<TranslateResponse, ErrorCode> {
    let target = require_target(target_code(request.target))?;
    let base = context.base_or(|| Base::https("translation.googleapis.com"));
    let body = serde_json::to_vec(&json!({
        "q": request.texts,
        "target": target,
        "format": "html",
    }))
    .map_err(|_| ErrorCode::TranslationInvalidResponse)?;
    let reply = http::post(Post {
        base: &base,
        route: context.route,
        path: "/language/translate/v2",
        headers: vec![(
            HeaderName::from_static("x-goog-api-key"),
            http::secret_header(context.key.expose_secret())?,
        )],
        body,
    })
    .await?;
    if reply.status != 200 {
        return Err(failure(reply.status, &reply.body));
    }
    let answer: Answer =
        serde_json::from_slice(&reply.body).map_err(|_| ErrorCode::TranslationInvalidResponse)?;
    let items = answer.data.translations;
    let detected_source = normalize_detected(
        items
            .iter()
            .find_map(|item| item.detected_source_language.clone()),
    );
    Ok(TranslateResponse {
        texts: items.into_iter().map(|item| item.translated_text).collect(),
        detected_source,
    })
}
