//! The translation channel behind Edge's built-in page translator. It is not a
//! service Microsoft offers to other programs: no key, no terms, and it can
//! change or stop at any time. It ignores `textType=html` (attributes come
//! back mangled), so segments travel in the tag-name form of `wire` and are
//! converted back to the HTML wire form before anything else sees them.
use serde::Deserialize;

use super::{
    Context, TranslateRequest, TranslateResponse,
    http::{self, Base, Post},
    microsoft, normalize_detected, require_target,
};
use crate::{
    ErrorCode,
    translate::{
        Piece,
        wire::{self, decode_tags, encode_tags},
    },
};

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

/// The HTML wire form closes every span with a bare `</span>`, so crossed or
/// unbalanced tags would come back as a different, valid nesting. Those answers
/// keep only their text, which `rewrite` then treats like any other mismatch.
fn to_html_wire(answer: &str) -> String {
    let pieces = decode_tags(answer);
    if wire::tags_balanced(&pieces) {
        wire::encode_pieces(&pieces)
    } else {
        let text: Vec<Piece> = pieces
            .into_iter()
            .filter(|piece| matches!(piece, Piece::Text(_)))
            .collect();
        wire::encode_pieces(&text)
    }
}

pub(super) async fn translate(
    context: &Context<'_>,
    request: &TranslateRequest<'_>,
) -> Result<TranslateResponse, ErrorCode> {
    let target = require_target(microsoft::target_code(request.target))?;
    let base = context.base_or(|| Base::https("edge.microsoft.com"));
    let tagged: Vec<String> = request
        .texts
        .iter()
        .map(|text| encode_tags(&wire::decode(text)))
        .collect();
    let body = serde_json::to_vec(&tagged).map_err(|_| ErrorCode::TranslationInvalidResponse)?;
    let reply = http::post(Post {
        base: &base,
        route: context.route,
        // `target` comes from a fixed table, so it needs no escaping.
        path: &format!("/translate/translatetext?from=&to={target}&isEnterpriseClient=false"),
        headers: Vec::new(),
        body,
    })
    .await?;
    if reply.status != 200 {
        return Err(http::keyless_status_error(reply.status));
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
            (Some(first), None) => texts.push(to_html_wire(&first.text)),
            _ => return Err(ErrorCode::TranslationInvalidResponse),
        }
    }
    Ok(TranslateResponse {
        texts,
        detected_source,
    })
}
