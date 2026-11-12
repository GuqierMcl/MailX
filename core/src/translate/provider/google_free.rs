//! The translation channel behind Google's web translate widget. It is not a
//! service Google offers to other programs: no key, no terms, and it can
//! change or stop at any time. It understands the HTML wire form as is.
//! The answer is `[["text","en"],..]` with `sl=auto`; a fixed source language
//! returns plain strings, so both shapes are read.
use hyper::header::{self, HeaderValue};
use serde::Deserialize;

use super::{
    Context, TranslateRequest, TranslateResponse, google,
    http::{self, Base, Post},
    normalize_detected, require_target,
};
use crate::ErrorCode;

#[derive(Deserialize)]
#[serde(untagged)]
enum Entry {
    Plain(String),
    // The second element is the detected source language.
    WithLanguage(Vec<String>),
}

pub(super) async fn translate(
    context: &Context<'_>,
    request: &TranslateRequest<'_>,
) -> Result<TranslateResponse, ErrorCode> {
    let target = require_target(google::target_code(request.target))?;
    let base = context.base_or(|| Base::https("translate.googleapis.com"));
    // The serializer is not Send, so it must not live across the await below.
    let body = {
        let mut form = url::form_urlencoded::Serializer::new(String::new());
        for text in request.texts {
            form.append_pair("q", text);
        }
        form.finish().into_bytes()
    };
    let reply = http::post(Post {
        base: &base,
        route: context.route,
        // `target` comes from a fixed table, so it needs no escaping.
        path: &format!("/translate_a/t?client=gtx&sl=auto&tl={target}&format=html"),
        headers: vec![(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        )],
        body,
    })
    .await?;
    if reply.status != 200 {
        return Err(http::keyless_status_error(reply.status));
    }
    let entries: Vec<Entry> =
        serde_json::from_slice(&reply.body).map_err(|_| ErrorCode::TranslationInvalidResponse)?;
    let mut detected = None;
    let mut texts = Vec::with_capacity(entries.len());
    for entry in entries {
        match entry {
            Entry::Plain(text) => texts.push(text),
            Entry::WithLanguage(mut parts) => {
                if parts.is_empty() {
                    return Err(ErrorCode::TranslationInvalidResponse);
                }
                if detected.is_none() && parts.len() > 1 {
                    detected = Some(parts.swap_remove(1));
                }
                texts.push(parts.swap_remove(0));
            }
        }
    }
    Ok(TranslateResponse {
        texts,
        detected_source: normalize_detected(detected),
    })
}
