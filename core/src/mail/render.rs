//! Pure MIME decoding and HTML preparation. This module never fetches resources.
//!
//! The parser can materialize the complete (at most 100 MiB) MIME message and
//! decoded parts before we retain 20 MiB of each body representation. The latter
//! is a retained-content limit, not a process-memory limit. HTML parsing likewise
//! has an input-byte budget rather than a claimed parser node-allocation limit.
use std::fmt;

use mail_parser::{Message, MessageParser, MessagePart, MimeHeaders, PartType};

mod css;
mod html;
mod images;

pub use html::{ResourceBinding, bind_numbered_resources, bind_resources, is_allowed_link};
pub use images::InlineImage;

pub const SANITIZER_VERSION: u32 = 1;
pub const MAX_MESSAGE_BYTES: usize = 100 * 1024 * 1024;
pub const MAX_BODY_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_MIME_DEPTH: usize = 30;
pub const MAX_INLINE_IMAGE_BYTES: usize = 50 * 1024 * 1024;
pub const MAX_DATA_IMAGE_URL_BYTES: usize = 1024 * 1024;

/// Deliberately no Debug/Serialize: message contents are not diagnostics or IPC.
#[derive(Clone, PartialEq, Eq)]
pub struct Rendered {
    pub plain_text: Option<String>,
    /// Only this renderer creates the structural resource attributes in this
    /// document. The host must call `bind_resources` before serving it.
    pub sanitized_html: Option<String>,
    pub links: Vec<String>,
    pub remote_images_blocked: u32,
    pub truncated: bool,
    pub parts: Vec<RenderedPart>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct RenderedPart {
    /// Stable pre-order entity index, including containers and nested messages.
    pub part_index: usize,
    pub content_id: Option<String>,
    pub filename: Option<String>,
    pub mime_type: String,
    pub size: u64,
    pub is_inline: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenderError {
    TooLarge,
    Protocol,
    InvalidImage,
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TooLarge => "message exceeds the supported size",
            Self::Protocol => "message structure cannot be processed",
            Self::InvalidImage => "image cannot be provided",
        })
    }
}

impl std::error::Error for RenderError {}

/// Parse real MIME body types, preserving only passive HTML and resource slots.
/// `message_id` identifies the owner's scope; actual URL authority and document
/// tokens belong to the host, and never come from the message itself.
pub fn render(raw: &[u8], message_id: &str) -> Result<Rendered, RenderError> {
    if message_id.is_empty() {
        return Err(RenderError::Protocol);
    }
    let message = parse(raw)?;
    let entities = entities(&message)?;
    let mut truncated = false;
    let mut plain_text = collect_body(&message, false, &mut truncated);
    let raw_html = collect_body(&message, true, &mut truncated);
    let mut links = Vec::new();
    let mut remote_images_blocked = 0;
    let sanitized_html = raw_html.map(|body| {
        let sanitized = html::sanitize(&body);
        links = sanitized.links;
        remote_images_blocked = sanitized.remote_images_blocked;
        if plain_text.is_none() {
            let text = html::plain_text(&sanitized.html);
            plain_text = Some(truncate(text, &mut truncated));
        }
        sanitized.html
    });
    let parts = entities
        .into_iter()
        .filter(|entity| !entity.is_body && !matches!(entity.part.body, PartType::Multipart(_)))
        .map(|entity| RenderedPart {
            part_index: entity.index,
            content_id: entity.part.content_id().map(normalize_cid),
            filename: entity.part.attachment_name().map(str::to_owned),
            mime_type: mime_type(entity.part),
            size: part_bytes(entity.part).len() as u64,
            is_inline: is_inline(entity.part),
        })
        .collect();
    Ok(Rendered {
        plain_text,
        sanitized_html,
        links,
        remote_images_blocked,
        truncated,
        parts,
    })
}

/// A list excerpt, counted in Unicode scalar values rather than UTF-8 bytes.
pub fn preview(text: Option<&str>) -> String {
    text.unwrap_or_default().chars().take(200).collect()
}

/// Re-read one inline entity from its owning raw MIME. A duplicate Content-ID
/// always resolves to the first inline entity in pre-order; another index with
/// that CID is refused rather than letting callers select a different payload.
pub fn inline_image(
    raw: &[u8],
    part_index: usize,
    content_id: &str,
) -> Result<InlineImage, RenderError> {
    let message = parse(raw)?;
    let entities = entities(&message)?;
    let cid = normalize_cid(content_id);
    let entity = entities
        .into_iter()
        .find(|entity| {
            !entity.is_body
                && is_inline(entity.part)
                && entity.part.content_id().map(normalize_cid).as_deref() == Some(cid.as_str())
        })
        .filter(|entity| entity.index == part_index)
        .ok_or(RenderError::InvalidImage)?;
    images::validate(part_bytes(entity.part), &mime_type(entity.part))
}

fn parse(raw: &[u8]) -> Result<Message<'_>, RenderError> {
    if raw.len() > MAX_MESSAGE_BYTES {
        return Err(RenderError::TooLarge);
    }
    let message = MessageParser::default()
        .parse(raw)
        .ok_or(RenderError::Protocol)?;
    if let Err(error) = entities(&message) {
        // mail-parser uses a stack for unencoded nested message/rfc822 parts.
        // Do not recursively drop an adversarially deep tree on rejection.
        let mut pending = vec![message];
        while let Some(mut message) = pending.pop() {
            for part in message.parts.drain(..) {
                if let PartType::Message(nested) = part.body {
                    pending.push(nested);
                }
            }
        }
        return Err(error);
    }
    Ok(message)
}

struct Entity<'a> {
    index: usize,
    part: &'a MessagePart<'a>,
    is_body: bool,
}

fn entities<'a>(message: &'a Message<'a>) -> Result<Vec<Entity<'a>>, RenderError> {
    let mut out = Vec::new();
    visit(message, 0, 1, true, &mut out)?;
    Ok(out)
}

fn visit<'a>(
    message: &'a Message<'a>,
    id: u32,
    depth: usize,
    root_message: bool,
    out: &mut Vec<Entity<'a>>,
) -> Result<(), RenderError> {
    if depth > MAX_MIME_DEPTH {
        return Err(RenderError::Protocol);
    }
    let part = message
        .parts
        .get(id as usize)
        .ok_or(RenderError::Protocol)?;
    if part.is_encoding_problem {
        return Err(RenderError::Protocol);
    }
    let mime = mime_type(part);
    if (mime.starts_with("multipart/") && !matches!(part.body, PartType::Multipart(_)))
        || (matches!(mime.as_str(), "message/rfc822" | "message/global")
            && !matches!(part.body, PartType::Message(_)))
    {
        return Err(RenderError::Protocol);
    }
    out.push(Entity {
        index: out.len(),
        part,
        is_body: root_message
            && ((message.text_body.contains(&id) && matches!(part.body, PartType::Text(_)))
                || (message.html_body.contains(&id) && matches!(part.body, PartType::Html(_)))),
    });
    match &part.body {
        PartType::Multipart(children) => {
            if children.is_empty() {
                return Err(RenderError::Protocol);
            }
            for child in children {
                visit(message, *child, depth + 1, root_message, out)?;
            }
        }
        PartType::Message(nested) => visit(nested, 0, depth + 1, false, out)?,
        _ => {}
    }
    Ok(())
}

fn collect_body(message: &Message<'_>, html: bool, truncated: &mut bool) -> Option<String> {
    let ids = if html {
        &message.html_body
    } else {
        &message.text_body
    };
    let mut result = None;
    for id in ids {
        let part = message.parts.get(*id as usize)?;
        // Do not call body_html/body_text: they synthesize the other format.
        let text = match &part.body {
            PartType::Html(value) if html => value.as_ref(),
            PartType::Text(value) if !html => value.as_ref(),
            _ => continue,
        };
        let joined = result.get_or_insert_with(String::new);
        if !joined.is_empty() && joined.len() < MAX_BODY_BYTES {
            joined.push('\n');
        }
        let available = MAX_BODY_BYTES.saturating_sub(joined.len());
        let mut end = text.len().min(available);
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        joined.push_str(&text[..end]);
        *truncated |= end < text.len();
    }
    result
}

fn truncate(mut value: String, truncated: &mut bool) -> String {
    if value.len() > MAX_BODY_BYTES {
        let mut end = MAX_BODY_BYTES;
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        value.truncate(end);
        *truncated = true;
    }
    value
}

fn mime_type(part: &MessagePart<'_>) -> String {
    part.content_type()
        .map(|content_type| {
            format!(
                "{}/{}",
                content_type.ctype(),
                content_type.subtype().unwrap_or("octet-stream")
            )
            .to_ascii_lowercase()
        })
        .unwrap_or_else(|| match part.body {
            PartType::Text(_) => "text/plain".to_owned(),
            PartType::Html(_) => "text/html".to_owned(),
            _ => "application/octet-stream".to_owned(),
        })
}

fn normalize_cid(cid: &str) -> String {
    cid.trim()
        .trim_start_matches('<')
        .trim_end_matches('>')
        .to_owned()
}

fn is_inline(part: &MessagePart<'_>) -> bool {
    if let Some(disposition) = part.content_disposition() {
        if disposition.ctype().eq_ignore_ascii_case("attachment") {
            return false;
        }
        if disposition.ctype().eq_ignore_ascii_case("inline") {
            return true;
        }
    }
    part.content_id().is_some() || matches!(part.body, PartType::InlineBinary(_))
}

fn part_bytes<'a>(part: &'a MessagePart<'a>) -> &'a [u8] {
    match &part.body {
        PartType::Text(value) | PartType::Html(value) => value.as_bytes(),
        PartType::Binary(value) | PartType::InlineBinary(value) => value.as_ref(),
        PartType::Message(message) => message.raw_message.as_ref(),
        PartType::Multipart(_) => &[],
    }
}

#[cfg(test)]
mod tests;
