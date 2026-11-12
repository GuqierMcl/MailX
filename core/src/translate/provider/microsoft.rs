//! Azure AI Translator v3 `POST /translate`, checked against
//! https://learn.microsoft.com/azure/ai-services/translator/text-translation/reference/v3/translate
//! The global endpoint is used with the resource region sent as a header, so a
//! regional resource works without a user-supplied URL.
use hyper::header::HeaderName;
use serde::Deserialize;
use serde_json::json;

use super::{
    Context, TranslateRequest, TranslateResponse,
    http::{self, Base, Post},
    normalize_detected, require_target,
};
use crate::{ErrorCode, translate::languages};

/// Every MailX target is a Translator code as is; `pt` means Brazilian
/// Portuguese there and `nb` is Norwegian Bokmal.
pub(super) fn target_code(code: &str) -> Option<&'static str> {
    languages::TARGETS
        .iter()
        .copied()
        .find(|target| *target == code)
}

#[derive(Deserialize)]
struct Item {
    translations: Vec<Translation>,
    #[serde(rename = "detectedLanguage")]
    detected_language: Option<Detected>,
}

#[derive(Deserialize)]
struct Translation {
    text: String,
}

#[derive(Deserialize)]
struct Detected {
    language: String,
}

pub(super) async fn translate(
    context: &Context<'_>,
    request: &TranslateRequest<'_>,
) -> Result<TranslateResponse, ErrorCode> {
    let target = require_target(target_code(request.target))?;
    let base = context.base_or(|| Base::https("api.cognitive.microsofttranslator.com"));
    let body = serde_json::to_vec(
        &request
            .texts
            .iter()
            .map(|text| json!({ "Text": text }))
            .collect::<Vec<_>>(),
    )
    .map_err(|_| ErrorCode::TranslationInvalidResponse)?;
    let mut headers = vec![(
        HeaderName::from_static("ocp-apim-subscription-key"),
        http::secret_header(context.key.expose_secret())?,
    )];
    if !context.microsoft_region.is_empty() {
        headers.push((
            HeaderName::from_static("ocp-apim-subscription-region"),
            http::secret_header(context.microsoft_region)?,
        ));
    }
    let reply = http::post(Post {
        base: &base,
        route: context.route,
        // `target` comes from a fixed table, so it needs no escaping.
        path: &format!("/translate?api-version=3.0&to={target}&textType=html"),
        headers,
        body,
    })
    .await?;
    match reply.status {
        200 => {}
        // Documented as "often: all free translations used".
        403 => return Err(ErrorCode::TranslationQuotaExceeded),
        408 => return Err(ErrorCode::TranslationUnavailable),
        status => return Err(http::common_status_error(status)),
    }
    let items: Vec<Item> =
        serde_json::from_slice(&reply.body).map_err(|_| ErrorCode::TranslationInvalidResponse)?;
    let detected_source = normalize_detected(
        items
            .iter()
            .find_map(|item| item.detected_language.as_ref().map(|d| d.language.clone())),
    );
    let mut texts = Vec::with_capacity(items.len());
    for item in items {
        let mut translations = item.translations.into_iter();
        match (translations.next(), translations.next()) {
            (Some(first), None) => texts.push(first.text),
            _ => return Err(ErrorCode::TranslationInvalidResponse),
        }
    }
    Ok(TranslateResponse {
        texts,
        detected_source,
    })
}
