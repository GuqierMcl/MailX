//! The composer's HTML boundary is deliberately smaller than the mail reader's.
//! Only this module derives the plain-text body and adds outgoing email styles.

use std::{borrow::Cow, collections::HashSet};

use ammonia::{Builder, UrlRelative};
use scraper::{ElementRef, Html, Node};
use url::Url;

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
];
const DROP_CONTENT: &[&str] = &[
    "script", "style", "iframe", "object", "embed", "template", "svg", "math", "noscript", "head",
    "title",
];

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
    let html = serialize(&document, Mode::Draft, None, usize::MAX);
    if html.len() > MAX_HTML_BYTES {
        return Err(AppError::validation("draft.bodyHtml"));
    }
    let text = plain_text(&document)?;
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
            ]
            .into_iter()
            .collect(),
        )
        .clean_content_tags(DROP_CONTENT.iter().copied().collect())
        .url_schemes(HashSet::from(["http", "https", "mailto"]))
        .url_relative(UrlRelative::Deny)
        .link_rel(None)
        .attribute_filter(|tag, attribute, value| match (tag, attribute) {
            ("a", "href") => allowed_href(value).map(Cow::Borrowed),
            ("ol", "start") => value
                .trim()
                .parse::<i64>()
                .ok()
                .map(|n| Cow::Owned(n.to_string())),
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
pub(crate) fn quote_cached_source(html: Option<&str>, links: &[String], text: &str) -> String {
    quote(html, Some(links), text)
}

#[cfg(test)]
fn quote_source(html: Option<&str>, text: &str) -> String {
    quote(html, None, text)
}

fn quote(html: Option<&str>, links: Option<&[String]>, text: &str) -> String {
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
        let output = sanitize(&serialize(&document, Mode::Quote, links, budget));
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
    let body = serialize(&document, Mode::Email, None, usize::MAX);
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"></head><body><div style=\"font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;font-size:14px;line-height:1.5;white-space:pre-wrap\">{body}</div></body></html>"
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Draft,
    Quote,
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
fn serialize(document: &Html, mode: Mode, links: Option<&[String]>, limit: usize) -> String {
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
            if mode == Mode::Quote && original == "img" {
                if let Some(alt) = element
                    .value()
                    .attr("alt")
                    .filter(|alt| !alt.trim().is_empty())
                {
                    full = !escape_into(
                        &format!("[图片：{alt}]"),
                        &mut output,
                        limit.saturating_sub(reserved),
                    );
                }
                continue;
            }
            let tag = canonical_tag(original);
            if mode == Mode::Quote && matches!(tag, "h1" | "h2" | "h3" | "h4" | "h5" | "h6") {
                open.push_str("<p><strong>");
                close.push_str("</strong></p>");
            } else if mode == Mode::Quote
                && matches!(tag, "div" | "td" | "th" | "pre")
                && !has_block_children(element)
            {
                open.push_str("<p>");
                close.push_str("</p>");
            } else if TAGS.contains(&tag) {
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
                if mode == Mode::Email {
                    let style = match tag {
                        "p" => Some("margin:0"),
                        "blockquote" => {
                            Some("margin:0 0 0 0.8ex;border-left:1px solid #ccc;padding-left:1ex")
                        }
                        "ul" | "ol" => Some("margin-top:0;margin-bottom:0"),
                        _ => None,
                    };
                    if let Some(style) = style {
                        open.push_str(&format!(" style=\"{style}\""));
                    }
                }
                open.push('>');
                if mode == Mode::Email && tag == "p" && !element.has_children() {
                    // Empty HTML paragraphs otherwise have zero height in mail
                    // clients, unlike the editor's visible empty paragraph.
                    open.push_str("<br>");
                }
                if tag != "br" {
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

fn plain_text(document: &Html) -> Result<String, AppError> {
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
                "p" => writer.flush(true)?,
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
            "p" => writer.flush(false)?,
            "br" => writer.flush(true)?,
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
            html.starts_with("<p><strong>标题</strong></p><p>甲</p><p>乙</p><p>丙</p>"),
            "{html}"
        );
        assert_eq!(
            normalize(&html).unwrap().text,
            "标题\n甲\n乙\n丙\n[图片：图 & <x>]"
        );
        assert!(!html.contains("img") && !html.contains("script") && !html.contains("table"));
    }

    #[test]
    fn cached_links_only_come_from_the_trusted_link_table() {
        let html = quote_cached_source(
            Some(
                "<p><a data-mailx-link='0' href='https://forged'>safe</a> <a data-mailx-link='7' title='https://forged'>missing</a></p>",
            ),
            &["https://real.example".into()],
            "",
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
            let html = quote_cached_source(Some(&format!("{leading}<p>正文仍然保留</p>")), &[], "");
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
}
