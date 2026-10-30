//! One attachment part of an outgoing message (docs/architecture/sending.md
//! §16.5).
//!
//! The file name goes out twice so every client finds it: RFC 2231
//! (`filename*=UTF-8''…`, continuations when long) in `Content-Disposition`
//! and RFC 2047 encoded words in the `name` parameter of `Content-Type`. Both
//! headers are assembled here as pure ASCII; lettre only folds them.

use base64::{Engine, engine::general_purpose::STANDARD};
use lettre::message::Body;
use lettre::message::SinglePart;
use lettre::message::header::{ContentTransferEncoding, Header, HeaderName, HeaderValue};

use crate::AppError;

/// A file to attach, read back from the blob store.
pub(crate) struct AttachmentData {
    pub file_name: String,
    pub media_type: String,
    pub bytes: Vec<u8>,
}

/// An inline picture the HTML body shows through `cid:`.
pub(crate) struct InlineImageData {
    /// Without angle brackets.
    pub content_id: String,
    pub file: AttachmentData,
}

#[derive(Clone)]
struct PartContentId(String);

#[derive(Clone)]
struct PartType(String);

#[derive(Clone)]
struct PartDisposition(String);

macro_rules! raw_header {
    ($type:ident, $name:literal) => {
        impl Header for $type {
            fn name() -> HeaderName {
                HeaderName::new_from_ascii_str($name)
            }

            fn parse(value: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
                Ok(Self(value.to_owned()))
            }

            fn display(&self) -> HeaderValue {
                // Pure ASCII by construction: the encoder only folds it.
                HeaderValue::new(Self::name(), self.0.clone())
            }
        }
    };
}

raw_header!(PartType, "Content-Type");
raw_header!(PartDisposition, "Content-Disposition");
raw_header!(PartContentId, "Content-ID");

pub(crate) fn part(attachment: &AttachmentData) -> Result<SinglePart, AppError> {
    build(attachment, "attachment", None)
}

/// Same name encoding as a regular attachment, with `Content-Disposition:
/// inline` and a `Content-ID` so `cid:` links resolve.
pub(crate) fn inline_part(image: &InlineImageData) -> Result<SinglePart, AppError> {
    build(&image.file, "inline", Some(&image.content_id))
}

fn build(
    attachment: &AttachmentData,
    disposition_type: &str,
    content_id: Option<&str>,
) -> Result<SinglePart, AppError> {
    let media_type = if token_pair(&attachment.media_type) {
        attachment.media_type.as_str()
    } else {
        "application/octet-stream"
    };
    let name = &attachment.file_name;
    let (content_type, disposition) = if name.is_ascii() && !name.chars().any(char::is_control) {
        let quoted = quote(name);
        (
            format!("{media_type}; name={quoted}"),
            format!("{disposition_type}; filename={quoted}"),
        )
    } else {
        (
            format!("{media_type}; name=\"{}\"", encoded_words(name)),
            format!(
                "{disposition_type}; {}; filename=\"{}\"",
                rfc2231("filename", name),
                encoded_words(name)
            ),
        )
    };
    let body = Body::new_with_encoding(attachment.bytes.clone(), ContentTransferEncoding::Base64)
        .map_err(|_| AppError::validation("attachments"))?;
    let builder = SinglePart::builder()
        .header(PartType(content_type))
        .header(PartDisposition(disposition));
    Ok(match content_id {
        Some(id) => builder.header(PartContentId(format!("<{id}>"))).body(body),
        None => builder.body(body),
    })
}

/// `type/subtype` made of RFC 2045 token characters only.
fn token_pair(value: &str) -> bool {
    let token = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$&^_.+-".contains(&b))
    };
    value
        .split_once('/')
        .is_some_and(|(a, b)| token(a) && token(b))
}

fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Splits `value` into pieces of at most `limit` bytes on character boundaries.
fn chunks(value: &str, limit: usize) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut start = 0;
    for (at, c) in value.char_indices() {
        if at + c.len_utf8() - start > limit {
            pieces.push(&value[start..at]);
            start = at;
        }
    }
    if start < value.len() {
        pieces.push(&value[start..]);
    }
    pieces
}

/// RFC 2047 encoded words (`=?utf-8?b?…?=`), each within the 75 character
/// limit, separated by spaces that decoders drop between adjacent words.
fn encoded_words(value: &str) -> String {
    chunks(value, 45)
        .into_iter()
        .map(|piece| format!("=?utf-8?b?{}?=", STANDARD.encode(piece)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn attr_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&byte)
}

fn percent(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        if attr_char(byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// RFC 2231 parameter: `name*=UTF-8''…`, or numbered continuations
/// (`name*0*=UTF-8''…; name*1*=…`) when it would not fit a folded line.
fn rfc2231(name: &str, value: &str) -> String {
    let single = percent(value);
    if single.len() <= 48 {
        return format!("{name}*=UTF-8''{single}");
    }
    chunks(value, 16)
        .into_iter()
        .enumerate()
        .map(|(index, piece)| {
            let charset = if index == 0 { "UTF-8''" } else { "" };
            format!("{name}*{index}*={charset}{}", percent(piece))
        })
        .collect::<Vec<_>>()
        .join("; ")
}
