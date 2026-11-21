//! Coarse presentation hints from the local, sanitized document. Image URLs
//! are used to group repeated slots, never to infer importance from the sender.
use std::collections::{HashMap, HashSet};

use cssparser::{Delimiter, Parser, ParserInput, parse_important};
use ego_tree::iter::Edge;
use scraper::{Html, Node, node::Element};

use super::html::{MAX_REMOTE_IMAGES, remote_slot_url};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub enum RemoteImageRole {
    Decorative,
    Content,
    Important,
}

#[derive(Clone)]
pub(crate) struct RemoteImageSlot {
    pub(crate) url: String,
    pub(crate) role: RemoteImageRole,
}

/// Retain the cascade among the few useful inline declarations. The CSS parser
/// skips separators in quoted strings and functions. Relative lengths remain unknown.
fn inline_hints(style: &str) -> HashMap<&str, (&str, bool)> {
    let mut hints = HashMap::new();
    let mut input = ParserInput::new(style);
    let mut parser = Parser::new(&mut input);
    while !parser.is_exhausted() {
        let _ = parser.parse_until_after(Delimiter::Semicolon, |decl| {
            let start = decl.position();
            decl.expect_ident()?;
            let name = decl.slice_from(start).trim();
            decl.expect_colon()?;
            let value = decl.parse_until_before(Delimiter::Bang, |value| {
                let start = value.position();
                while value.next().is_ok() {}
                Ok(value.slice_from(start).trim())
            })?;
            let important = decl.try_parse(parse_important).is_ok();
            decl.expect_exhausted()?;
            if matches!(
                name,
                "width" | "height" | "display" | "visibility" | "opacity"
            ) && (important || !matches!(hints.get(name), Some((_, true))))
            {
                hints.insert(name, (value, important));
            }
            Ok::<_, cssparser::ParseError<'_, ()>>(())
        });
    }
    hints
}

fn pixels(value: &str) -> Option<f32> {
    let value = value.trim();
    let number = value
        .strip_suffix("px")
        .unwrap_or(value)
        .parse::<f32>()
        .ok()?;
    (number.is_finite() && number >= 0.0).then_some(number)
}

#[derive(Copy, Clone, Default)]
struct Visibility {
    suppressed: bool,
    invisible: bool,
}

impl Visibility {
    fn hidden(self) -> bool {
        self.suppressed || self.invisible
    }
}

struct ImageHints {
    width: Option<f32>,
    height: Option<f32>,
    has_alt: bool,
    qr_label: bool,
    hidden: bool,
}

impl ImageHints {
    fn decorative(&self) -> bool {
        self.hidden
            || self.width.is_some_and(|w| w <= 2.0)
            || self.height.is_some_and(|h| h <= 2.0)
            || matches!((self.width, self.height), (Some(w), Some(h)) if w <= 8.0 && h <= 8.0)
    }

    fn role(&self, sole_image_in_sparse_body: bool) -> RemoteImageRole {
        if self.decorative() {
            return RemoteImageRole::Decorative;
        }
        let cover =
            matches!((self.width, self.height), (Some(w), Some(h)) if w >= 300.0 && h >= 160.0);
        let qr = self.qr_label
            && match (self.width, self.height) {
                (Some(w), Some(h)) => w.min(h) >= 24.0 && w.max(h) <= w.min(h) * 1.25,
                _ => true,
            };
        let substantial = self.width.is_some_and(|w| w >= 160.0)
            || self.height.is_some_and(|h| h >= 160.0)
            || (self.width.is_none() && self.height.is_none() && self.has_alt);
        if cover || qr || (sole_image_in_sparse_body && substantial) {
            RemoteImageRole::Important
        } else {
            RemoteImageRole::Content
        }
    }
}

fn qr_label(alt: &str) -> bool {
    alt.contains("二维码")
        || alt.contains("二維碼")
        || alt
            .split(|c: char| !c.is_ascii_alphanumeric())
            .any(|word| word.eq_ignore_ascii_case("qr") || word.eq_ignore_ascii_case("qrcode"))
}

/// Preserve the remote index ordering of the document protocol, including
/// hidden slots. This only measures author-declared layout, never a live DOM.
pub(crate) fn remote_image_slots(html: &str) -> Vec<RemoteImageSlot> {
    const TEXT_THRESHOLD: usize = 40;
    let document = Html::parse_fragment(html);
    let mut visibility = vec![Visibility::default()];
    let mut candidates = Vec::new();
    let mut text_length = 0;
    let mut other_visible_image = false;
    for edge in document.tree.root().traverse() {
        match edge {
            Edge::Open(node) => match node.value() {
                Node::Text(text)
                    if !visibility.last().unwrap().hidden() && text_length < TEXT_THRESHOLD =>
                {
                    text_length += text
                        .text
                        .chars()
                        .filter(|c| !c.is_whitespace())
                        .take(TEXT_THRESHOLD - text_length)
                        .count();
                }
                Node::Element(element) => {
                    let style = inline_hints(element.attr("style").unwrap_or(""));
                    let property = |name: &str| style.get(name).map(|&(value, _)| value);
                    let mut visible = *visibility.last().unwrap();
                    visible.suppressed |=
                        matches!(element.name(), "head" | "style" | "script" | "template")
                            || element.attr("hidden").is_some()
                            || property("display").is_some_and(|v| v.eq_ignore_ascii_case("none"))
                            || property("opacity")
                                .is_some_and(|v| v.trim_end_matches('%').parse::<f32>() == Ok(0.0));
                    match property("visibility") {
                        Some(v) if v.eq_ignore_ascii_case("visible") => visible.invisible = false,
                        Some(v)
                            if v.eq_ignore_ascii_case("hidden")
                                || v.eq_ignore_ascii_case("collapse") =>
                        {
                            visible.invisible = true
                        }
                        _ => {}
                    }
                    visibility.push(visible);
                    if element.name() == "img" {
                        let dimension = |name: &str| {
                            property(name)
                                .and_then(pixels)
                                .or_else(|| element.attr(name).and_then(pixels))
                        };
                        let alt = element.attr("alt").unwrap_or("").trim();
                        let hints = ImageHints {
                            width: dimension("width"),
                            height: dimension("height"),
                            has_alt: !alt.is_empty(),
                            qr_label: qr_label(alt),
                            hidden: visible.hidden(),
                        };
                        if candidates.len() < MAX_REMOTE_IMAGES
                            && let Some(url) = remote_slot_url(element)
                        {
                            candidates.push((url, hints));
                        } else if has_local_source(element) && !hints.decorative() {
                            other_visible_image = true;
                        }
                    }
                }
                _ => {}
            },
            Edge::Close(node) if node.value().is_element() => {
                visibility.pop();
            }
            Edge::Close(_) => {}
        }
    }
    let distinct: HashSet<_> = candidates
        .iter()
        .filter(|(_, hints)| !hints.decorative())
        .map(|(url, _)| url)
        .collect();
    let sole = !other_visible_image && text_length < TEXT_THRESHOLD && distinct.len() == 1;
    candidates
        .into_iter()
        .map(|(url, hints)| RemoteImageSlot {
            url,
            role: hints.role(sole),
        })
        .collect()
}

fn has_local_source(element: &Element) -> bool {
    element.attr("data-mailx-cid").is_some() || element.attr("src").is_some()
}
