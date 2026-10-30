//! The composer's HTML boundary is deliberately smaller than the mail reader's.
//! Only this module derives the plain-text body and adds outgoing email styles.

use std::{borrow::Cow, collections::HashSet};

use ammonia::{Builder, UrlRelative};
use scraper::{ElementRef, Html, Node};
use url::Url;

use crate::SupportedLocale;
use crate::error::AppError;

const MAX_HTML_BYTES: usize = 2 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = super::validate::MAX_BODY_BYTES;
const MAX_QUOTE_BYTES: usize = 512 * 1024;

const TAGS: &[&str] = &[
    "p",
    "br",
    "strong",
    "b",
    "em",
    "i",
    "u",
    "s",
    "strike",
    "del",
    "a",
    "ul",
    "ol",
    "li",
    "blockquote",
    "h1",
    "h2",
    "h3",
    "span",
    "mark",
    "img",
];
const MAX_ALT_CHARS: usize = 256;
const MAX_IMAGE_EDGE: u32 = 10_000;
const DROP_CONTENT: &[&str] = &[
    "script", "style", "iframe", "object", "embed", "template", "svg", "math", "noscript", "head",
    "title",
];

// Storage contains palette names only. Email colors are fixed and independent
// of the editor's light/dark theme; arbitrary CSS never crosses this boundary.
const TEXT_COLORS: &[(&str, &str)] = &[
    ("gray", "#6b7280"),
    ("red", "#dc2626"),
    ("orange", "#ea580c"),
    ("yellow", "#ca8a04"),
    ("green", "#16a34a"),
    ("blue", "#2563eb"),
    ("purple", "#9333ea"),
    ("pink", "#db2777"),
];
const HIGHLIGHT_COLORS: &[(&str, &str)] = &[
    ("gray", "#e5e7eb"),
    ("red", "#fecaca"),
    ("yellow", "#fef08a"),
    ("green", "#bbf7d0"),
    ("blue", "#bfdbfe"),
];

fn palette_color<'a>(palette: &'a [(&str, &str)], name: &str) -> Option<&'a str> {
    palette
        .iter()
        .find_map(|(key, color)| (*key == name).then_some(*color))
}

fn alignment(value: &str) -> Option<&str> {
    matches!(value, "center" | "right").then_some(value)
}

/// Returns the Content-ID (without angle brackets) when `src` is a `cid:`
/// reference in the exact format core generates: 32 lowercase hex digits, `@`,
/// then a domain. Anything else, including other `cid:` shapes, is rejected.
pub(crate) fn cid_of(src: &str) -> Option<&str> {
    let id = src.strip_prefix("cid:")?;
    let (local, domain) = id.split_once('@')?;
    let domain_ok = !domain.is_empty()
        && domain.len() <= 253
        && domain
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-'));
    (local.len() == 32
        && local
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && domain_ok)
        .then_some(id)
}

fn image_alt(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_ALT_CHARS)
        .collect()
}

fn image_edge(value: &str) -> Option<u32> {
    let value = value.trim();
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    value
        .parse::<u32>()
        .ok()
        .filter(|n| (1..=MAX_IMAGE_EDGE).contains(n))
}

/// Content-IDs referenced by `<img src="cid:…">` in already normalized
/// draft HTML, in order of first appearance and without duplicates.
pub(crate) fn referenced_cids(html: &str) -> Vec<String> {
    let document = Html::parse_fragment(html);
    let mut seen = HashSet::new();
    let mut found = Vec::new();
    for node in document.tree.nodes() {
        let Some(element) = ElementRef::wrap(node) else {
            continue;
        };
        if element.value().name() != "img" {
            continue;
        }
        if let Some(cid) = element.value().attr("src").and_then(cid_of)
            && seen.insert(cid)
        {
            found.push(cid.to_owned());
        }
    }
    found
}

/// Final HTML and text for sending: `<img>` elements whose Content-ID has no
/// stored image are removed, and the text writes `[alt]` or `[file name]`.
/// `images` maps Content-IDs to file names for every row that will be sent.
pub(crate) fn outgoing_body(
    html: &str,
    images: &[(String, String)],
) -> Result<NormalizedBody, AppError> {
    let known: HashSet<&str> = images.iter().map(|(cid, _)| cid.as_str()).collect();
    let document = Html::parse_fragment(html);
    let html = serialize(&document, Mode::Draft, None, usize::MAX, Some(&known));
    let document = Html::parse_fragment(&html);
    let names = |cid: &str| {
        images
            .iter()
            .find_map(|(id, name)| (id == cid).then(|| name.clone()))
    };
    let text = plain_text(&document, &names)?;
    Ok(NormalizedBody { html, text })
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct NormalizedBody {
    pub html: String,
    pub text: String,
}

pub(crate) fn normalize(html: &str) -> Result<NormalizedBody, AppError> {
    if html.len() > MAX_HTML_BYTES || html.contains('\0') {
        return Err(AppError::validation("draft.bodyHtml"));
    }
    let clean = sanitize(html);
    // Canonical aliases keep a save/reopen cycle stable with the editor schema.
    let document = Html::parse_fragment(&clean);
    let html = serialize(&document, Mode::Draft, None, usize::MAX, None);
    if html.len() > MAX_HTML_BYTES {
        return Err(AppError::validation("draft.bodyHtml"));
    }
    let text = plain_text(&document, &|_| None)?;
    Ok(NormalizedBody { html, text })
}

fn sanitize(html: &str) -> String {
    Builder::new()
        .tags(TAGS.iter().copied().collect())
        .generic_attributes(HashSet::new())
        .tag_attributes(
            [
                ("a", HashSet::from(["href"])),
                ("ol", HashSet::from(["start"])),
                ("p", HashSet::from(["data-align"])),
                ("h1", HashSet::from(["data-align"])),
                ("h2", HashSet::from(["data-align"])),
                ("h3", HashSet::from(["data-align"])),
                ("span", HashSet::from(["data-color"])),
                ("mark", HashSet::from(["data-highlight"])),
                ("img", HashSet::from(["src", "alt", "width", "height"])),
            ]
            .into_iter()
            .collect(),
        )
        .clean_content_tags(DROP_CONTENT.iter().copied().collect())
        .url_schemes(HashSet::from(["http", "https", "mailto", "cid"]))
        .url_relative(UrlRelative::Deny)
        .link_rel(None)
        .attribute_filter(|tag, attribute, value| match (tag, attribute) {
            ("a", "href") => allowed_href(value).map(Cow::Borrowed),
            ("ol", "start") => value
                .trim()
                .parse::<i64>()
                .ok()
                .map(|n| Cow::Owned(n.to_string())),
            ("p" | "h1" | "h2" | "h3", "data-align") => alignment(value).map(Cow::Borrowed),
            ("span", "data-color") if palette_color(TEXT_COLORS, value).is_some() => {
                Some(Cow::Borrowed(value))
            }
            ("mark", "data-highlight") if palette_color(HIGHLIGHT_COLORS, value).is_some() => {
                Some(Cow::Borrowed(value))
            }
            ("img", "src") => cid_of(value).map(|_| Cow::Borrowed(value)),
            ("img", "alt") => Some(Cow::Owned(image_alt(value))),
            ("img", "width" | "height") => image_edge(value).map(|n| Cow::Owned(n.to_string())),
            _ => None,
        })
        .clean(html)
        .to_string()
}

fn allowed_href(value: &str) -> Option<&str> {
    if value.chars().any(char::is_control) {
        return None;
    }
    let value = value.trim();
    let url = Url::parse(value).ok()?;
    match url.scheme() {
        "http" | "https" if url.host_str().is_some() => Some(value),
        "mailto" if !url.path().is_empty() => Some(value),
        _ => None,
    }
}

pub(crate) fn plain_text_to_html(text: &str) -> String {
    bounded_plain_text_to_html(text, usize::MAX)
}

fn bounded_plain_text_to_html(text: &str, limit: usize) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut html = String::new();
    for line in text.split('\n') {
        if limit.saturating_sub(html.len()) < "<p></p>".len() {
            break;
        }
        html.push_str("<p>");
        let complete = escape_into(line, &mut html, limit - "</p>".len());
        html.push_str("</p>");
        if !complete {
            break;
        }
    }
    html
}

/// Reading HTML stores links in a trusted, separate table. Restore only those
/// indexed links before the independent composer sanitizer runs.
pub(crate) fn quote_cached_source(
    html: Option<&str>,
    links: &[String],
    text: &str,
    locale: SupportedLocale,
) -> String {
    quote(html, Some(links), text, locale)
}

#[cfg(test)]
fn quote_source(html: Option<&str>, text: &str) -> String {
    quote(html, None, text, SupportedLocale::ZhCn)
}

fn quote(
    html: Option<&str>,
    links: Option<&[String]>,
    text: &str,
    locale: SupportedLocale,
) -> String {
    let Some(html) = html else {
        // Escaping trusted plain text directly avoids parsing a potentially
        // five-times-larger intermediate full of HTML entities.
        return bounded_plain_text_to_html(truncate(text, MAX_QUOTE_BYTES), MAX_QUOTE_BYTES);
    };
    // The quote budget applies after dropping layout/resources. A large style
    // block or inline image near the beginning must not hide the text after it.
    // The cached reader body has its own, larger input budget.
    let document = Html::parse_fragment(truncate(html, crate::mail::render::MAX_BODY_BYTES));
    let mut budget = MAX_QUOTE_BYTES;
    loop {
        let output = sanitize(&serialize(
            &document,
            Mode::Quote(locale),
            links,
            budget,
            None,
        ));
        if output.len() <= MAX_QUOTE_BYTES {
            return output;
        }
        // HTML repair can insert closing tags. Re-render with room for those;
        // never byte-truncate serialized markup or split a UTF-8 code point.
        budget = budget.saturating_sub(output.len() - MAX_QUOTE_BYTES + 32);
    }
}

/// Input has already crossed `normalize`; styles here are fixed application
/// constants and never inherited from draft HTML or an original email.
pub(crate) fn email_document(fragment: &str) -> String {
    let document = Html::parse_fragment(fragment);
    let body = serialize(&document, Mode::Email, None, usize::MAX, None);
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"></head><body><div style=\"font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;font-size:14px;line-height:1.5;white-space:pre-wrap\">{body}</div></body></html>"
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Draft,
    Quote(SupportedLocale),
    Email,
}

fn canonical_tag(tag: &str) -> &str {
    match tag {
        "b" => "strong",
        "i" => "em",
        "strike" | "del" => "s",
        _ => tag,
    }
}

fn has_block_children(element: ElementRef<'_>) -> bool {
    element.child_elements().any(|child| {
        matches!(
            child.value().name(),
            "p" | "div"
                | "blockquote"
                | "ul"
                | "ol"
                | "table"
                | "h1"
                | "h2"
                | "h3"
                | "h4"
                | "h5"
                | "h6"
        )
    })
}

fn block_tag(tag: &str) -> bool {
    matches!(
        tag,
        "p" | "div"
            | "blockquote"
            | "ul"
            | "ol"
            | "li"
            | "table"
            | "tbody"
            | "thead"
            | "tfoot"
            | "tr"
            | "td"
            | "th"
            | "pre"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
    )
}

/// Indentation between block tags is source formatting, while spaces inside a
/// paragraph (including between inline marks) are text the user actually wrote.
fn layout_whitespace(
    text: &str,
    parent: Option<ElementRef<'_>>,
    before: Option<ElementRef<'_>>,
    after: Option<ElementRef<'_>>,
) -> bool {
    text.chars()
        .all(|character| character.is_ascii_whitespace())
        && (parent.is_some_and(|element| matches!(element.value().name(), "ul" | "ol"))
            || before.is_some_and(|element| block_tag(element.value().name()))
            || after.is_some_and(|element| block_tag(element.value().name())))
}

/// Iterative traversal also bounds attacker-controlled nesting. Reserving all
/// open closing tags lets quoted source stop at a strict serialized byte limit.
///
/// With `known`, an `<img>` whose Content-ID is not in the set is dropped.
fn serialize(
    document: &Html,
    mode: Mode,
    links: Option<&[String]>,
    limit: usize,
    known: Option<&HashSet<&str>>,
) -> String {
    let quoting = matches!(mode, Mode::Quote(_));
    let mut output = String::new();
    let mut reserved = 0usize;
    let mut full = false;
    let mut pending = vec![(document.tree.root(), None::<String>)];
    while let Some((node, close)) = pending.pop() {
        if let Some(close) = close {
            reserved -= close.len();
            output.push_str(&close);
            continue;
        }
        if full {
            continue;
        }
        if let Node::Text(text) = node.value() {
            if layout_whitespace(
                &text.text,
                node.parent().and_then(ElementRef::wrap),
                node.prev_sibling().and_then(ElementRef::wrap),
                node.next_sibling().and_then(ElementRef::wrap),
            ) {
                continue;
            }
            full = !escape_into(&text.text, &mut output, limit.saturating_sub(reserved));
            continue;
        }
        let mut open = String::new();
        let mut close = String::new();
        if let Some(element) = ElementRef::wrap(node) {
            let original = element.value().name();
            if DROP_CONTENT.contains(&original) {
                continue;
            }
            if let Mode::Quote(locale) = mode
                && original == "img"
            {
                if let Some(alt) = element
                    .value()
                    .attr("alt")
                    .filter(|alt| !alt.trim().is_empty())
                {
                    full = !escape_into(
                        &locale.image_alt(alt),
                        &mut output,
                        limit.saturating_sub(reserved),
                    );
                }
                continue;
            }
            if original == "img" {
                // Only reachable outside quoting; a missing or foreign `src`
                // removes the whole element rather than leaving a stub.
                let cid = element.value().attr("src").and_then(cid_of);
                if cid.is_none_or(|cid| known.is_some_and(|known| !known.contains(cid))) {
                    continue;
                }
            }
            let tag = if quoting && matches!(original, "h4" | "h5" | "h6") {
                "h3"
            } else {
                canonical_tag(original)
            };
            // Quoted mail keeps its heading structure, but never imports the
            // sender's alignment, colors, highlights, or palette attributes.
            let align = (!quoting && matches!(tag, "p" | "h1" | "h2" | "h3"))
                .then(|| element.value().attr("data-align").and_then(alignment))
                .flatten();
            let color = (!quoting && tag == "span")
                .then(|| element.value().attr("data-color"))
                .flatten()
                .filter(|name| palette_color(TEXT_COLORS, name).is_some());
            let highlight = (!quoting && tag == "mark")
                .then(|| element.value().attr("data-highlight"))
                .flatten()
                .filter(|name| palette_color(HIGHLIGHT_COLORS, name).is_some());
            if quoting && matches!(tag, "div" | "td" | "th" | "pre") && !has_block_children(element)
            {
                open.push_str("<p>");
                close.push_str("</p>");
            } else if TAGS.contains(&tag)
                && (tag != "span" || color.is_some())
                && (tag != "mark" || highlight.is_some())
            {
                open.push('<');
                open.push_str(tag);
                let href = if tag == "a" {
                    let cached = links.and_then(|links| {
                        element
                            .value()
                            .attr("data-mailx-link")
                            .and_then(|index| index.parse::<usize>().ok())
                            .and_then(|index| links.get(index))
                            .map(String::as_str)
                    });
                    // In cached HTML, only the trusted link table grants a URL.
                    cached
                        .or_else(|| {
                            links
                                .is_none()
                                .then(|| element.value().attr("href"))
                                .flatten()
                        })
                        .and_then(allowed_href)
                } else {
                    None
                };
                if let Some(href) = href {
                    open.push_str(" href=\"");
                    escape_into(href, &mut open, usize::MAX);
                    open.push('"');
                }
                if tag == "ol"
                    && let Some(start) = element
                        .value()
                        .attr("start")
                        .and_then(|v| v.parse::<i64>().ok())
                {
                    open.push_str(&format!(" start=\"{start}\""));
                }
                if tag == "img" {
                    open.push_str(" src=\"");
                    escape_into(
                        element.value().attr("src").unwrap_or_default(),
                        &mut open,
                        usize::MAX,
                    );
                    open.push('"');
                    if let Some(alt) = element.value().attr("alt") {
                        open.push_str(" alt=\"");
                        escape_into(&image_alt(alt), &mut open, usize::MAX);
                        open.push('"');
                    }
                    for attribute in ["width", "height"] {
                        if let Some(n) = element.value().attr(attribute).and_then(image_edge) {
                            open.push_str(&format!(" {attribute}=\"{n}\""));
                        }
                    }
                }
                if mode == Mode::Draft {
                    for (attribute, value) in [
                        ("data-align", align),
                        ("data-color", color),
                        ("data-highlight", highlight),
                    ] {
                        if let Some(value) = value {
                            open.push_str(&format!(" {attribute}=\"{value}\""));
                        }
                    }
                }
                if mode == Mode::Email {
                    let mut style = match tag {
                        "p" => "margin:0",
                        "h1" => "margin:0;font-size:20px;font-weight:bold",
                        "h2" => "margin:0;font-size:18px;font-weight:bold",
                        "h3" => "margin:0;font-size:16px;font-weight:bold",
                        "blockquote" => {
                            "margin:0 0 0 0.8ex;border-left:1px solid #ccc;padding-left:1ex"
                        }
                        "ul" | "ol" => "margin-top:0;margin-bottom:0",
                        "img" => "max-width: 100%; height: auto",
                        _ => "",
                    }
                    .to_owned();
                    if let Some(align) = align {
                        style.push_str(&format!(";text-align:{align}"));
                    }
                    if let Some(color) = color.and_then(|name| palette_color(TEXT_COLORS, name)) {
                        style = format!("color:{color}");
                    }
                    if let Some(color) =
                        highlight.and_then(|name| palette_color(HIGHLIGHT_COLORS, name))
                    {
                        style = format!("background-color:{color};color:inherit");
                    }
                    if !style.is_empty() {
                        open.push_str(&format!(" style=\"{style}\""));
                    }
                }
                open.push('>');
                if mode == Mode::Email
                    && matches!(tag, "p" | "h1" | "h2" | "h3")
                    && !element.has_children()
                {
                    // Empty HTML paragraphs otherwise have zero height in mail
                    // clients, unlike the editor's visible empty paragraph.
                    open.push_str("<br>");
                }
                if !matches!(tag, "br" | "img") {
                    close = format!("</{tag}>");
                }
            }
        }
        if open.len().saturating_add(close.len())
            > limit.saturating_sub(output.len()).saturating_sub(reserved)
        {
            full = true;
            continue;
        }
        output.push_str(&open);
        reserved += close.len();
        pending.push((node, Some(close)));
        pending.extend(node.children().rev().map(|child| (child, None)));
    }
    output
}

fn truncate(value: &str, max: usize) -> &str {
    let mut end = max.min(value.len());
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

/// Returns false when the byte budget has been reached.
fn escape_into(value: &str, output: &mut String, limit: usize) -> bool {
    for character in value.chars() {
        let mut buffer = [0; 4];
        let escaped = match character {
            '&' => "&amp;",
            '<' => "&lt;",
            '>' => "&gt;",
            '"' => "&quot;",
            '\'' => "&#39;",
            _ => character.encode_utf8(&mut buffer),
        };
        if escaped.len() > limit.saturating_sub(output.len()) {
            return false;
        }
        output.push_str(escaped);
    }
    true
}

#[derive(Default)]
struct TextWriter {
    output: String,
    line: String,
    lines: usize,
    meaningful: bool,
    quote_depth: usize,
    lists: Vec<Option<i64>>,
    items: Vec<ListItem>,
}

struct ListItem {
    marker: String,
    depth: usize,
    first_line: bool,
}

impl TextWriter {
    fn text(&mut self, value: &str) -> Result<(), AppError> {
        for character in value.chars() {
            if character == '\n' {
                self.flush(true)?;
            } else {
                self.line.push(if character == '\u{a0}' {
                    ' '
                } else {
                    character
                });
                self.meaningful |= !character.is_whitespace();
                self.check(self.line.len())?;
            }
        }
        Ok(())
    }

    fn check(&self, additional: usize) -> Result<(), AppError> {
        if additional > MAX_TEXT_BYTES.saturating_sub(self.output.len()) {
            Err(AppError::validation("draft.bodyText"))
        } else {
            Ok(())
        }
    }

    fn flush(&mut self, force: bool) -> Result<(), AppError> {
        if !force && self.line.is_empty() {
            return Ok(());
        }
        let (indent, marker) = self.items.last().map_or((0, ""), |item| {
            if item.first_line {
                (item.depth.saturating_sub(1) * 2, item.marker.as_str())
            } else {
                (item.depth * 2, "")
            }
        });
        let prefix = self
            .quote_depth
            .saturating_mul(2)
            .saturating_add(indent)
            .saturating_add(marker.len());
        self.check(
            prefix
                .saturating_add(self.line.len())
                .saturating_add(usize::from(self.lines > 0)),
        )?;
        if self.lines > 0 {
            self.output.push('\n');
        }
        for _ in 0..self.quote_depth {
            self.output.push_str("> ");
        }
        for _ in 0..indent {
            self.output.push(' ');
        }
        self.output.push_str(marker);
        self.output.push_str(&self.line);
        self.line.clear();
        self.lines += 1;
        if let Some(item) = self.items.last_mut() {
            item.first_line = false;
        }
        Ok(())
    }
}

fn plain_text(
    document: &Html,
    image_name: &dyn Fn(&str) -> Option<String>,
) -> Result<String, AppError> {
    let mut writer = TextWriter::default();
    let mut pending = vec![(document.tree.root(), false)];
    while let Some((node, exiting)) = pending.pop() {
        let Some(element) = ElementRef::wrap(node) else {
            if let Node::Text(text) = node.value() {
                if !layout_whitespace(
                    &text.text,
                    node.parent().and_then(ElementRef::wrap),
                    node.prev_sibling().and_then(ElementRef::wrap),
                    node.next_sibling().and_then(ElementRef::wrap),
                ) {
                    writer.text(&text.text)?;
                }
            } else {
                pending.extend(node.children().rev().map(|child| (child, false)));
            }
            continue;
        };
        let tag = element.value().name();
        if exiting {
            match tag {
                "p" | "h1" | "h2" | "h3" => writer.flush(true)?,
                "blockquote" => {
                    writer.flush(false)?;
                    writer.quote_depth -= 1;
                }
                "ul" | "ol" => {
                    writer.flush(false)?;
                    writer.lists.pop();
                }
                "li" => {
                    writer.flush(writer.items.last().is_some_and(|item| item.first_line))?;
                    writer.items.pop();
                }
                "a" => {
                    if let Some(href) = element.value().attr("href") {
                        let label = element.text().collect::<String>();
                        let label = label.trim();
                        if label.is_empty() {
                            writer.text(href)?;
                        } else if label != href {
                            writer.text(&format!(" <{href}>"))?;
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        match tag {
            "p" | "h1" | "h2" | "h3" => writer.flush(false)?,
            "br" => writer.flush(true)?,
            "img" if element.value().attr("src").and_then(cid_of).is_some() => {
                let alt = element
                    .value()
                    .attr("alt")
                    .map(str::trim)
                    .filter(|alt| !alt.is_empty());
                let label = alt.map(str::to_owned).or_else(|| {
                    element
                        .value()
                        .attr("src")
                        .and_then(cid_of)
                        .and_then(image_name)
                });
                writer.text(&format!("[{}]", label.as_deref().unwrap_or("image")))?;
            }
            "blockquote" => {
                writer.flush(false)?;
                writer.quote_depth += 1;
            }
            "ul" | "ol" => {
                writer.flush(false)?;
                writer.lists.push((tag == "ol").then(|| {
                    element
                        .value()
                        .attr("start")
                        .and_then(|v| v.parse::<i64>().ok())
                        .unwrap_or(1)
                }));
            }
            "li" => {
                writer.flush(false)?;
                let marker = match writer.lists.last_mut().and_then(Option::as_mut) {
                    Some(next) => {
                        let marker = format!("{next}. ");
                        *next = next.saturating_add(1);
                        marker
                    }
                    None => "- ".to_owned(),
                };
                writer.items.push(ListItem {
                    marker,
                    depth: writer.lists.len().max(1),
                    first_line: true,
                });
            }
            _ => {}
        }
        pending.push((node, true));
        pending.extend(node.children().rev().map(|child| (child, false)));
    }
    writer.flush(false)?;
    Ok(if writer.meaningful {
        writer.output
    } else {
        String::new()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composer_sanitizer_has_a_small_independent_allowlist() {
        let body = normalize(r#"<p id="x" style="color:red" onclick="attack()"><b>bold</b><i>italic</i><del>gone</del><img src="https://bad" alt="lost"></p><script>attack()</script><style>evil</style><iframe>hidden</iframe><div>kept</div>"#).unwrap();
        assert_eq!(
            body.html,
            "<p><strong>bold</strong><em>italic</em><s>gone</s></p>kept"
        );
        assert_eq!(body.text, "bolditalicgone\nkept");
        assert_eq!(normalize(&body.html).unwrap(), body);
    }

    #[test]
    fn links_require_safe_absolute_addresses_and_only_numeric_list_start_survives() {
        for href in [
            "javascript:alert(1)",
            "data:text/html,x",
            "/inbox",
            "//example.com",
            "#part",
            "file:///tmp/x",
            "mailto:",
            "https://",
        ] {
            assert!(
                !normalize(&format!("<p><a href=\"{href}\">text</a></p>"))
                    .unwrap()
                    .html
                    .contains("href="),
                "{href}"
            );
        }
        let body = normalize(r#"<p><a href="https://example.com" target="_blank">site</a> <a href="mailto:a@example.com">mail</a></p><ol start="3" reversed><li>three</li><li>four</li></ol><ol start="no"><li>one</li></ol>"#).unwrap();
        assert_eq!(
            body.text,
            "site <https://example.com> mail <mailto:a@example.com>\n3. three\n4. four\n1. one"
        );
        assert!(
            !body.html.contains("target")
                && !body.html.contains("reversed")
                && !body.html.contains("start=\"no\"")
        );
        assert_eq!(
            normalize("<a href='https://example.com'>https://example.com</a>")
                .unwrap()
                .text,
            "https://example.com"
        );
    }

    #[test]
    fn paragraphs_breaks_nested_lists_and_quotes_have_readable_plain_text() {
        let body = normalize("<p>first<br>second</p><p></p><ul><li><p>one</p><ol start='2'><li><p>nested</p></li></ol></li><li>two</li></ul><blockquote><p>quoted</p><blockquote><p>twice</p></blockquote></blockquote>").unwrap();
        assert_eq!(
            body.text,
            "first\nsecond\n\n- one\n  2. nested\n- two\n> quoted\n> > twice"
        );
        assert_eq!(normalize("<p>a</p><p></p>").unwrap().text, "a\n");
        for empty in [
            "",
            "<p></p>",
            "<p> &nbsp; </p>",
            "<ul><li><p></p></li></ul>",
            "<blockquote><p></p></blockquote>",
        ] {
            assert_eq!(normalize(empty).unwrap().text, "");
        }
        assert_eq!(
            normalize("<a href='https://example.com'></a>")
                .unwrap()
                .text,
            "https://example.com"
        );
    }

    #[test]
    fn legacy_plain_text_escapes_markup_and_preserves_lines() {
        let html = plain_text_to_html("<hi>&\r\n\r\n最后\r");
        assert_eq!(html, "<p>&lt;hi&gt;&amp;</p><p></p><p>最后</p><p></p>");
        assert_eq!(normalize(&html).unwrap().text, "<hi>&\n\n最后\n");
    }

    #[test]
    fn ordinary_text_preserves_leading_repeated_spaces_and_tabs() {
        let body = normalize("<p>  A  B\tC </p><p><strong>D</strong>  <em>E</em></p><blockquote><p>  F\n\tG</p></blockquote>").unwrap();
        assert_eq!(body.text, "  A  B\tC \nD  E\n>   F\n> \tG");
        let document = email_document(&body.html);
        assert!(document.contains("white-space:pre-wrap"));
        assert!(document.contains("<p style=\"margin:0\">  A  B\tC </p>"));
        assert_eq!(normalize(&body.html).unwrap(), body);
        assert_eq!(
            normalize(&plain_text_to_html("  A  B\tC \n D"))
                .unwrap()
                .text,
            "  A  B\tC \n D"
        );
    }

    #[test]
    fn source_indentation_between_blocks_does_not_add_blank_lines() {
        let body = normalize(
            "\n<p>A</p>\n \t<ul>\n <li><p>  B\tC</p> \n <ul>\n<li>D</li>\n</ul>\n</li>\n</ul>\n",
        )
        .unwrap();
        assert_eq!(body.text, "A\n-   B\tC\n  - D");
        assert_eq!(
            body.html,
            "<p>A</p><ul><li><p>  B\tC</p><ul><li>D</li></ul></li></ul>"
        );
    }

    #[test]
    fn quoted_mail_downgrades_layout_and_keeps_alt_text() {
        let html = quote_source(
            Some(
                "<h2>标题</h2><table><tr><td>甲</td><td><p>乙</p><p>丙</p></td></tr></table><p><img src='https://bad' alt='图 &amp; &lt;x&gt;'></p><script>no</script>",
            ),
            "fallback",
        );
        assert!(
            html.starts_with("<h2>标题</h2><p>甲</p><p>乙</p><p>丙</p>"),
            "{html}"
        );
        assert_eq!(
            normalize(&html).unwrap().text,
            "标题\n甲\n乙\n丙\n[图片：图 & <x>]"
        );
        assert!(!html.contains("img") && !html.contains("script") && !html.contains("table"));
    }

    #[test]
    fn localized_image_labels_escape_alt_and_do_not_reinterpret_it() {
        for (locale, expected) in [
            (SupportedLocale::ZhCn, "[图片：{{date}} <b>&]"),
            (SupportedLocale::En, "[Image: {{date}} <b>&]"),
        ] {
            let html = quote_cached_source(
                Some("<p><img alt='{{date}} &lt;b&gt;&amp;'></p>"),
                &[],
                "",
                locale,
            );
            assert_eq!(normalize(&html).unwrap().text, expected);
            assert!(html.contains("&lt;b&gt;&amp;"));
            assert!(!html.contains("<img") && !html.contains("<b>"));
        }
    }

    #[test]
    fn localized_image_labels_remain_within_the_quote_budget() {
        let source = format!(
            "<blockquote><p><img alt='{}'></p></blockquote>",
            "x".repeat(MAX_QUOTE_BYTES + 32),
        );
        for locale in [SupportedLocale::ZhCn, SupportedLocale::En] {
            let html = quote_cached_source(Some(&source), &[], "", locale);
            assert!(html.len() <= MAX_QUOTE_BYTES);
            assert!(html.ends_with("</p></blockquote>"));
            assert!(normalize(&html).is_ok());
        }
    }

    #[test]
    fn cached_links_only_come_from_the_trusted_link_table() {
        let html = quote_cached_source(
            Some(
                "<p><a data-mailx-link='0' href='https://forged'>safe</a> <a data-mailx-link='7' title='https://forged'>missing</a></p>",
            ),
            &["https://real.example".into()],
            "",
            SupportedLocale::ZhCn,
        );
        assert_eq!(
            normalize(&html).unwrap().text,
            "safe <https://real.example> missing"
        );
        assert!(!html.contains("forged") && !html.contains("data-mailx"));
    }

    #[test]
    fn discarded_large_styles_and_images_do_not_consume_the_quote_budget() {
        for leading in [
            format!("<style>{}</style>", "x".repeat(MAX_QUOTE_BYTES + 32)),
            format!(
                "<img src=\"data:image/png;base64,{}\">",
                "x".repeat(MAX_QUOTE_BYTES + 32)
            ),
        ] {
            let html = quote_cached_source(
                Some(&format!("{leading}<p>正文仍然保留</p>")),
                &[],
                "",
                SupportedLocale::ZhCn,
            );
            assert_eq!(html, "<p>正文仍然保留</p>");
        }
    }

    #[test]
    fn quoted_source_is_bounded_even_when_escaping_expands_the_input() {
        for source in [
            "&".repeat(MAX_QUOTE_BYTES),
            "中".repeat(MAX_QUOTE_BYTES / 2),
        ] {
            let html = quote_source(None, &source);
            assert!(html.len() <= MAX_QUOTE_BYTES);
            assert!(html.ends_with("</p>"));
            assert!(normalize(&html).is_ok());
        }
        let html = quote_source(
            Some(&format!(
                "<blockquote><p>{}</p></blockquote>",
                "中".repeat(MAX_QUOTE_BYTES / 2)
            )),
            "",
        );
        assert!(html.len() <= MAX_QUOTE_BYTES);
        assert!(html.ends_with("</p></blockquote>"));
    }

    #[test]
    fn input_and_derived_text_limits_are_independent() {
        assert_eq!(
            normalize(&"x".repeat(MAX_HTML_BYTES + 1))
                .unwrap_err()
                .field
                .as_deref(),
            Some("draft.bodyHtml")
        );
        assert_eq!(
            normalize("\0").unwrap_err().field.as_deref(),
            Some("draft.bodyHtml")
        );
        assert_eq!(
            normalize(&"x".repeat(MAX_TEXT_BYTES + 1))
                .unwrap_err()
                .field
                .as_deref(),
            Some("draft.bodyText")
        );
        assert!(normalize(&"x".repeat(MAX_TEXT_BYTES)).is_ok());
    }

    #[test]
    fn outgoing_document_only_adds_fixed_inline_styles() {
        let body =
            normalize("<p>text</p><blockquote><p>quote</p></blockquote><ol><li>item</li></ol>")
                .unwrap();
        let html = email_document(&body.html);
        assert!(html.contains("<meta charset=\"utf-8\">"));
        assert!(html.contains("<p style=\"margin:0\">"));
        assert!(html.contains("margin:0 0 0 0.8ex;border-left:1px solid #ccc;padding-left:1ex"));
        assert!(html.contains("margin-top:0;margin-bottom:0"));
        assert!(!html.contains("<style") && !html.contains("class=") && !html.contains("src="));
    }

    #[test]
    fn outgoing_empty_paragraphs_keep_their_line_height() {
        let body = normalize("<p>first</p><p></p><p></p><p>last</p>").unwrap();
        assert_eq!(body.text, "first\n\n\nlast");
        assert!(email_document(&body.html).contains(
            "<p style=\"margin:0\">first</p><p style=\"margin:0\"><br></p><p style=\"margin:0\"><br></p><p style=\"margin:0\">last</p>"
        ));
        assert_eq!(body.html, "<p>first</p><p></p><p></p><p>last</p>");
    }

    #[test]
    fn headings_and_alignment_keep_plain_text_and_use_fixed_email_styles() {
        let body = normalize("<h1 data-align='center'>First</h1><h2 data-align='right'>Second</h2><h3>Third</h3><p data-align='center'>Body</p><ul><li><p data-align='right'>Item</p></li></ul><blockquote><h2 data-align='center'>Quote</h2></blockquote>").unwrap();
        assert_eq!(body.text, "First\nSecond\nThird\nBody\n- Item\n> Quote");
        assert_eq!(normalize(&body.html).unwrap(), body);
        let html = email_document(&body.html);
        for expected in [
            "<h1 style=\"margin:0;font-size:20px;font-weight:bold;text-align:center\">First</h1>",
            "<h2 style=\"margin:0;font-size:18px;font-weight:bold;text-align:right\">Second</h2>",
            "<h3 style=\"margin:0;font-size:16px;font-weight:bold\">Third</h3>",
            "<p style=\"margin:0;text-align:center\">Body</p>",
            "<li><p style=\"margin:0;text-align:right\">Item</p></li>",
        ] {
            assert!(html.contains(expected), "{expected}: {html}");
        }
        assert!(!html.contains("data-"));
        assert_eq!(
            normalize("<h1></h1><h2>  </h2><h3>\t</h3>").unwrap().text,
            ""
        );
        assert!(email_document("<h1></h1>").contains("font-weight:bold\"><br></h1>"));
    }

    #[test]
    fn palette_names_are_the_only_color_and_highlight_values() {
        for (name, expected) in [
            ("gray", "#6b7280"),
            ("red", "#dc2626"),
            ("orange", "#ea580c"),
            ("yellow", "#ca8a04"),
            ("green", "#16a34a"),
            ("blue", "#2563eb"),
            ("purple", "#9333ea"),
            ("pink", "#db2777"),
        ] {
            let body = normalize(&format!(
                "<p><span data-color='{name}' style='color:black' class='external'>Color</span></p>"
            ))
            .unwrap();
            assert_eq!(
                body.html,
                format!("<p><span data-color=\"{name}\">Color</span></p>")
            );
            assert_eq!(body.text, "Color");
            assert!(
                email_document(&body.html)
                    .contains(&format!("<span style=\"color:{expected}\">Color</span>"))
            );
        }
        for (name, expected) in [
            ("gray", "#e5e7eb"),
            ("red", "#fecaca"),
            ("yellow", "#fef08a"),
            ("green", "#bbf7d0"),
            ("blue", "#bfdbfe"),
        ] {
            let body = normalize(&format!("<p><span data-color='purple'><mark data-highlight='{name}'><strong>Both</strong></mark></span></p>")).unwrap();
            assert_eq!(body.text, "Both");
            assert_eq!(normalize(&body.html).unwrap(), body);
            let html = email_document(&body.html);
            assert!(html.contains(&format!("<span style=\"color:#9333ea\"><mark style=\"background-color:{expected};color:inherit\"><strong>Both</strong></mark></span>")));
            assert!(!html.contains("data-"));
        }
    }

    #[test]
    fn arbitrary_attributes_and_invalid_palette_wrappers_are_removed() {
        for value in [
            "",
            "RED",
            " red",
            "red ",
            "#dc2626",
            "rgb(1,2,3)",
            "var(--red)",
            "red;background:url(https://tracker)",
        ] {
            let body = normalize(&format!("<p><span data-color='{value}' style='color:red'><strong>Keep</strong></span><mark data-highlight='{value}'>Text</mark></p>")).unwrap();
            assert_eq!(body.html, "<p><strong>Keep</strong>Text</p>", "{value}");
            assert_eq!(body.text, "KeepText");
        }
        for value in [
            "left",
            "justify",
            "CENTER",
            "center ",
            "right;position:fixed",
        ] {
            let body = normalize(&format!(
                "<h2 data-align='{value}'>Title</h2><p data-align='{value}'>Body</p>"
            ))
            .unwrap();
            assert_eq!(body.html, "<h2>Title</h2><p>Body</p>", "{value}");
        }
        assert_eq!(normalize("<p data-color='red' align='right' style='text-align:right'>Text<span data-highlight='yellow'>A</span><mark data-color='red'>B</mark><mark data-highlight='orange'>C</mark></p><ul data-align='center'><li data-align='right'>Item</li></ul>").unwrap().html, "<p>TextABC</p><ul><li>Item</li></ul>");
    }

    #[test]
    fn quoted_mail_keeps_heading_levels_but_discards_all_source_styling() {
        let html = quote_source(
            Some(
                "<h1 data-align='right' style='color:red;text-align:right'>One</h1><h2 align='center'>Two</h2><h3>Three</h3><h4>Four</h4><h5>Five</h5><h6>Six</h6><p data-align='center'><span data-color='red' style='color:#dc2626'><mark data-highlight='yellow' style='background:yellow'><strong>Formatted</strong></mark></span></p>",
            ),
            "",
        );
        assert_eq!(
            html,
            "<h1>One</h1><h2>Two</h2><h3>Three</h3><h3>Four</h3><h3>Five</h3><h3>Six</h3><p><strong>Formatted</strong></p>"
        );
        assert_eq!(
            normalize(&html).unwrap().text,
            "One\nTwo\nThree\nFour\nFive\nSix\nFormatted"
        );
        assert!(!html.contains("data-") && !html.contains("style") && !html.contains("align"));
    }

    const CID: &str = "cid:0123456789abcdef0123456789abcdef@mail.example.com";

    #[test]
    fn inline_images_keep_only_core_content_ids() {
        let body = normalize(&format!(
            "<p>a<img src=\"{CID}\" alt=\"pic\" width=\"120\" height=\"80\" onerror=\"x()\" style=\"color:red\"></p>"
        ))
        .unwrap();
        assert_eq!(
            body.html,
            format!("<p>a<img src=\"{CID}\" alt=\"pic\" width=\"120\" height=\"80\"></p>")
        );
        assert_eq!(body.text, "a[pic]");
        assert_eq!(normalize(&body.html).unwrap(), body);
        for src in [
            "data:image/png;base64,AAAA",
            "https://example.com/a.png",
            "http://example.com/a.png",
            "file:///etc/passwd",
            "cid:short@example.com",
            "cid:0123456789ABCDEF0123456789abcdef@example.com",
            "cid:0123456789abcdef0123456789abcdef",
            "cid:0123456789abcdef0123456789abcdef@",
            "cid:0123456789abcdef0123456789abcdef@bad domain",
            "cid:0123456789abcdef0123456789abcdef0@example.com",
            "cid:my-logo",
            "",
        ] {
            let body = normalize(&format!("<p>x<img src=\"{src}\" alt=\"a\"></p>")).unwrap();
            assert_eq!(body.html, "<p>x</p>", "{src:?}");
        }
        assert_eq!(
            normalize("<p>x<img alt=\"a\"></p>").unwrap().html,
            "<p>x</p>"
        );
    }

    #[test]
    fn inline_image_attributes_are_bounded() {
        let long = "字".repeat(300);
        let body = normalize(&format!("<img src=\"{CID}\" alt=\"{long}\">")).unwrap();
        assert!(body.html.contains(&format!("alt=\"{}\"", "字".repeat(256))));
        for value in ["0", "10001", "-5", "1.5", "12px", "auto", "99999999999", ""] {
            let body = normalize(&format!(
                "<img src=\"{CID}\" width=\"{value}\" height=\"{value}\">"
            ))
            .unwrap();
            assert!(
                !body.html.contains("width") && !body.html.contains("height"),
                "{value:?}"
            );
        }
        let body = normalize(&format!("<img src=\"{CID}\" width=\"1\" height=\"10000\">")).unwrap();
        assert!(body.html.contains("width=\"1\" height=\"10000\""));
    }

    #[test]
    fn email_images_get_responsive_style_and_quoting_still_drops_them() {
        let body = normalize(&format!("<p><img src=\"{CID}\"></p>")).unwrap();
        assert!(email_document(&body.html).contains(&format!(
            "<img src=\"{CID}\" style=\"max-width: 100%; height: auto\">"
        )));
        let quoted = quote_source(
            Some(&format!("<p>t</p><img src=\"{CID}\" alt=\"gone\">")),
            "",
        );
        assert!(!quoted.contains("<img") && !quoted.contains("cid:"));
    }

    #[test]
    fn referenced_cids_are_ordered_and_deduplicated() {
        let a = "0123456789abcdef0123456789abcdef@x.io";
        let b = "fedcba9876543210fedcba9876543210@x.io";
        let html = format!(
            "<p><img src=\"cid:{b}\"><img src=\"cid:{a}\"><img src=\"cid:{b}\"><img src=\"https://x/y.png\"></p>"
        );
        assert_eq!(referenced_cids(&html), [b, a]);
    }

    #[test]
    fn outgoing_body_removes_dangling_images_and_names_the_rest() {
        let a = "0123456789abcdef0123456789abcdef@x.io";
        let b = "fedcba9876543210fedcba9876543210@x.io";
        let c = "00000000000000000000000000000000@x.io";
        let html = normalize(&format!(
            "<p>hi<img src=\"cid:{a}\"><img src=\"cid:{b}\" alt=\"alt text\"><img src=\"cid:{c}\" alt=\"dangling\"></p>"
        ))
        .unwrap()
        .html;
        let out = outgoing_body(
            &html,
            &[
                (a.to_owned(), "cat.png".to_owned()),
                (b.to_owned(), "dog.png".to_owned()),
            ],
        )
        .unwrap();
        assert!(!out.html.contains(c));
        assert!(out.html.contains(a) && out.html.contains(b));
        assert_eq!(out.text, "hi[cat.png][alt text]");
    }
}
