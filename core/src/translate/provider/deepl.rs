//! DeepL API v2 `POST /v2/translate`, checked against
//! https://developers.deepl.com/api-reference/translate/request-translation
//! The target codes `ZH-HANT` and `NB` and the item limit could not be
//! confirmed from the published pages (UNVERIFIED).
use hyper::header::AUTHORIZATION;
use serde::Deserialize;
use serde_json::json;

use super::{
    Context, TranslateRequest, TranslateResponse,
    http::{self, Base, Post},
    normalize_detected, require_target,
};
use crate::ErrorCode;

/// Free-plan keys end in `:fx` and use their own host.
const FREE_KEY_SUFFIX: &str = ":fx";

pub(super) fn target_code(code: &str) -> Option<&'static str> {
    // Bare EN, PT and ZH are not accepted as targets; a variant is required.
    Some(match code {
        "zh-Hans" => "ZH-HANS",
        "zh-Hant" => "ZH-HANT",
        "en" => "EN-US",
        "pt" => "PT-BR",
        "ja" => "JA",
        "ko" => "KO",
        "fr" => "FR",
        "de" => "DE",
        "es" => "ES",
        "it" => "IT",
        "ru" => "RU",
        "nl" => "NL",
        "pl" => "PL",
        "tr" => "TR",
        "ar" => "AR",
        "id" => "ID",
        "uk" => "UK",
        "sv" => "SV",
        "da" => "DA",
        "fi" => "FI",
        "cs" => "CS",
        "el" => "EL",
        "hu" => "HU",
        "ro" => "RO",
        "bg" => "BG",
        "nb" => "NB",
        _ => return None,
    })
}

pub(super) fn host_for_key(key: &str) -> &'static str {
    if key.ends_with(FREE_KEY_SUFFIX) {
        "api-free.deepl.com"
    } else {
        "api.deepl.com"
    }
}

#[derive(Deserialize)]
struct Answer {
    translations: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    text: String,
    detected_source_language: Option<String>,
}

pub(super) async fn translate(
    context: &Context<'_>,
    request: &TranslateRequest<'_>,
) -> Result<TranslateResponse, ErrorCode> {
    let target = require_target(target_code(request.target))?;
    let key = context.key.expose_secret();
    let base = context.base_or(|| Base::https(host_for_key(key)));
    let body = serde_json::to_vec(&json!({
        "text": request.texts,
        "target_lang": target,
        "tag_handling": "html",
    }))
    .map_err(|_| ErrorCode::TranslationInvalidResponse)?;
    let reply = http::post(Post {
        base: &base,
        route: context.route,
        path: "/v2/translate",
        headers: vec![(
            AUTHORIZATION,
            http::secret_header(&format!("DeepL-Auth-Key {key}"))?,
        )],
        body,
    })
    .await?;
    match reply.status {
        200 => {}
        456 => return Err(ErrorCode::TranslationQuotaExceeded),
        // 529 is DeepL's "too many requests" alongside 429.
        529 => return Err(ErrorCode::TranslationRateLimited),
        status => return Err(http::common_status_error(status)),
    }
    let answer: Answer =
        serde_json::from_slice(&reply.body).map_err(|_| ErrorCode::TranslationInvalidResponse)?;
    let detected_source = normalize_detected(
        answer
            .translations
            .iter()
            .find_map(|item| item.detected_source_language.clone()),
    );
    Ok(TranslateResponse {
        texts: answer
            .translations
            .into_iter()
            .map(|item| item.text)
            .collect(),
        detected_source,
    })
}
