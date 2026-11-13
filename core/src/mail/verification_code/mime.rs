//! Local MIME adapter for the verification-code experiment. No resources are fetched.

use html5ever::QualName;
use mail_parser::{Message, MimeHeaders, PartType};
use scraper::{Html, Node, node::Element};

use super::{Detection, DetectionError, MAX_SUBJECT_BYTES, MAX_TEXT_BYTES, detect};
use crate::mail::render;

/// Raw input budget, before MIME parsing and transfer/charset decoding.
pub const MAX_EML_BYTES: usize = 1024 * 1024;

/// Detect a code in a complete local RFC 5322 message. Attachments and nested
/// messages are excluded. Real text/plain parts take precedence over HTML,
/// matching the reader's text source; an empty plain alternative falls back to
/// HTML. This is not a parser for partial IMAP previews.
///
/// HTML handling removes common quote containers and explicit inline hiding,
/// then uses the reader's sanitizer and text conversion. It does not compute
/// CSS layout or fetch stylesheets; arbitrary CSS hiding is not recognized.
pub fn detect_eml(raw: &[u8]) -> Result<Detection, DetectionError> {
    if raw.len() > MAX_EML_BYTES {
        return Err(DetectionError::InputTooLarge);
    }
    if !raw.windows(2).any(|bytes| bytes == b"\n\n")
        && !raw.windows(4).any(|bytes| bytes == b"\r\n\r\n")
    {
        return Err(DetectionError::InvalidMessage);
    }
    let message = render::parse(raw).map_err(|_| DetectionError::InvalidMessage)?;
    if message
        .parts
        .first()
        .is_none_or(|part| part.headers.is_empty())
    {
        return Err(DetectionError::InvalidMessage);
    }
    let subject = message.subject().unwrap_or_default();
    if subject.len() > MAX_SUBJECT_BYTES {
        return Err(DetectionError::InputTooLarge);
    }
    // Check both representations before choosing one. Silently truncating one
    // alternative could turn multiple candidates into a misleading unique code.
    let plain = body(&message, false)?;
    let html = body(&message, true)?;
    let text = if !plain.trim().is_empty() {
        plain
    } else if !html.is_empty() {
        visible_text(&html)
    } else {
        String::new()
    };
    detect(subject, &text)
}

fn body(message: &Message<'_>, html: bool) -> Result<String, DetectionError> {
    let ids = if html {
        &message.html_body
    } else {
        &message.text_body
    };
    let mut text = String::new();
    for id in ids {
        let part = message
            .parts
            .get(*id as usize)
            .ok_or(DetectionError::InvalidMessage)?;
        if message.attachments.contains(id)
            || part.attachment_name().is_some()
            || part
                .content_disposition()
                .is_some_and(|value| value.ctype().eq_ignore_ascii_case("attachment"))
        {
            continue;
        }
        let value = match &part.body {
            PartType::Text(value) if !html => value,
            PartType::Html(value) if html => value,
            _ => continue,
        };
        let separator = usize::from(!text.is_empty());
        if value.len() > MAX_TEXT_BYTES.saturating_sub(text.len() + separator) {
            return Err(DetectionError::InputTooLarge);
        }
        if separator > 0 {
            text.push('\n');
        }
        text.push_str(value);
    }
    Ok(text)
}

fn visible_text(html: &str) -> String {
    let mut document = Html::parse_fragment(html);
    let removed: Vec<_> = document
        .tree
        .root()
        .descendants()
        .filter_map(|node| {
            node.value()
                .as_element()
                .filter(|element| excluded(element))
                .map(|_| node.id())
        })
        .collect();
    for id in removed {
        // Preserve a boundary, so removing hidden or quoted text cannot join
        // previously separated digit runs into a new candidate.
        let br = document
            .tree
            .orphan(Node::Element(Element::new(
                QualName::new(None, "http://www.w3.org/1999/xhtml".into(), "br".into()),
                vec![],
            )))
            .id();
        document.tree.get_mut(id).unwrap().insert_id_before(br);
        document.tree.get_mut(id).unwrap().detach();
    }
    let filtered = document.root_element().inner_html();
    render::html_plain_text(&render::sanitize_document(&filtered).html)
}

fn excluded(element: &Element) -> bool {
    if matches!(
        element.name(),
        "blockquote" | "script" | "style" | "head" | "title" | "template" | "svg" | "noscript"
    ) || element.attr("hidden").is_some()
        || element
            .attr("aria-hidden")
            .is_some_and(|value| value.eq_ignore_ascii_case("true"))
    {
        return true;
    }
    const QUOTES: &[&str] = &[
        "gmail_quote",
        "gmail_extra",
        "yahoo_quoted",
        "protonmail_quote",
        "tutanota_quote",
        "zmail_extra",
        "mailx-quote",
    ];
    if element.attr("class").is_some_and(|value| {
        value
            .split_ascii_whitespace()
            .any(|class| QUOTES.contains(&class))
    }) || element
        .attr("id")
        .is_some_and(|value| QUOTES.contains(&value))
    {
        return true;
    }
    element.attr("style").is_some_and(|style| {
        style.split(';').any(|declaration| {
            let Some((property, value)) = declaration.split_once(':') else {
                return false;
            };
            let value = value.split('!').next().unwrap_or_default().trim();
            match property.trim().to_ascii_lowercase().as_str() {
                "display" => value.eq_ignore_ascii_case("none"),
                "visibility" => {
                    value.eq_ignore_ascii_case("hidden") || value.eq_ignore_ascii_case("collapse")
                }
                "mso-hide" => value.eq_ignore_ascii_case("all"),
                _ => false,
            }
        })
    })
}
