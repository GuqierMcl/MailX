use std::collections::HashSet;

use html5ever::QualName;
use scraper::{Html, Node, node::Text};
use url::Url;

use super::{css, images};

const CID_SLOT: &str = "data-mailx-cid";
const LINK_SLOT: &str = "data-mailx-link";
const REMOTE_SLOT: &str = "data-mailx-remote";
const DATA_SLOT: &str = "data-mailx-data";
const BODY_MARK: &str = "data-mailx-body";

pub(crate) struct Sanitized {
    pub html: String,
    pub links: Vec<String>,
    pub remote_images_blocked: u32,
}

/// These values can only originate in renderer-created DOM attributes. The host
/// resolves them to an owner- and document-token-scoped URL, never by replacing
/// matching text strings in the message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceBinding<'a> {
    Part { content_id: &'a str },
    Link { index: usize },
    Remote { index: usize },
}

/// Bind a trusted renderer result at serving time. Unresolved slots are removed.
/// Host-generated URLs must use one of Tauri's two supported protocol origins and
/// the matching resource route. Email text is never inspected or substituted.
pub fn bind_resources(
    sanitized_html: &str,
    resolve: impl FnMut(ResourceBinding<'_>) -> Option<String>,
) -> String {
    bind(sanitized_html, false, resolve)
}

/// Bind images while replacing link slots with passive, one-based text labels.
/// The trusted parent presents the corresponding database URLs. No email anchor
/// retains an href, and literal email text cannot create or alter a link slot.
pub fn bind_numbered_resources(
    sanitized_html: &str,
    resolve: impl FnMut(ResourceBinding<'_>) -> Option<String>,
) -> String {
    bind(sanitized_html, true, resolve)
}

fn bind(
    sanitized_html: &str,
    numbered_links: bool,
    mut resolve: impl FnMut(ResourceBinding<'_>) -> Option<String>,
) -> String {
    let mut document = Html::parse_fragment(sanitized_html);
    let ids: Vec<_> = document
        .tree
        .root()
        .descendants()
        .map(|node| node.id())
        .collect();
    let mut remote_index = 0;
    for id in ids {
        let mut node = document.tree.get_mut(id).unwrap();
        let Node::Element(element) = node.value() else {
            continue;
        };
        let remote = element.name() == "img" && element.attr(REMOTE_SLOT).is_some();
        if remote {
            if let Some(url) = resolve(ResourceBinding::Remote {
                index: remote_index,
            })
            .filter(|url| {
                valid_resource_url(url, true)
                    && Url::parse(url).is_ok_and(|url| url.path().starts_with("/remote/"))
            }) {
                set_attr(element, "src", &url);
            }
            remote_index += 1;
        }
        let slot = match element.name() {
            "img" => element
                .attr(CID_SLOT)
                .map(str::to_owned)
                .map(|cid| (cid, true)),
            "a" => element
                .attr(LINK_SLOT)
                .map(str::to_owned)
                .map(|index| (index, false)),
            _ => None,
        };
        element
            .attrs
            .retain(|(name, _)| !name.local.starts_with("data-mailx-"));
        if numbered_links && element.name() == "a" {
            element
                .attrs
                .retain(|(name, _)| name.local.as_ref() != "href");
        }
        let Some((value, part)) = slot else { continue };
        if numbered_links && !part {
            if let Some(number) = value
                .parse::<usize>()
                .ok()
                .and_then(|index| index.checked_add(1))
            {
                node.append(Node::Text(scraper::node::Text {
                    text: format!(" [{number}]").into(),
                }));
            }
            continue;
        }
        let resolved = if part {
            resolve(ResourceBinding::Part { content_id: &value })
        } else {
            value
                .parse::<usize>()
                .ok()
                .and_then(|index| resolve(ResourceBinding::Link { index }))
        };
        if let Some(url) = resolved.filter(|url| valid_resource_url(url, part)) {
            set_attr(element, if part { "src" } else { "href" }, &url);
        }
    }
    document.root_element().inner_html()
}

fn valid_resource_url(value: &str, part: bool) -> bool {
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    let authority = (url.scheme() == "mailx-body" && url.host_str() == Some("localhost"))
        || (url.scheme() == "http" && url.host_str() == Some("mailx-body.localhost"));
    authority
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url
            .path()
            .starts_with(if part && url.path().starts_with("/remote/") {
                "/remote/"
            } else if part {
                "/part/"
            } else {
                "/link/"
            })
}

pub fn is_allowed_link(value: &str) -> bool {
    canonical_link(value).is_some()
}

fn canonical_link(value: &str) -> Option<String> {
    if value.chars().any(char::is_control) {
        return None;
    }
    let url = Url::parse(value.trim()).ok()?;
    match url.scheme() {
        "http" | "https" if url.host_str().is_some() => Some(url.to_string()),
        "mailto" if !url.path().is_empty() => Some(url.to_string()),
        _ => None,
    }
}

pub const MAX_REMOTE_IMAGES: usize = 32;

fn remote_url(value: &str) -> Option<String> {
    if value.len() > 8192 || value.chars().any(char::is_control) {
        return None;
    }
    let value = value.trim();
    let url = Url::parse(&if value.starts_with("//") {
        format!("https:{value}")
    } else {
        value.to_owned()
    })
    .ok()?;
    (matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.to_string())
}

/// Only read from a versioned sanitizer result; these slots never reach the WebView.
pub fn remote_images(html: &str) -> Vec<String> {
    Html::parse_fragment(html)
        .tree
        .root()
        .descendants()
        .filter_map(|node| {
            let Node::Element(element) = node.value() else {
                return None;
            };
            (element.name() == "img")
                .then(|| element.attr(REMOTE_SLOT))
                .flatten()
                .and_then(remote_url)
        })
        .take(MAX_REMOTE_IMAGES)
        .collect()
}

pub(super) fn is_remote(value: &str) -> bool {
    // HTML's URL parser removes ASCII tabs/newlines before recognizing schemes.
    let normalized: String = value
        .trim()
        .chars()
        .filter(|c| !matches!(c, '\t' | '\r' | '\n'))
        .collect();
    normalized.starts_with("//")
        || Url::parse(&normalized).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
}

/// Fragment parsing discards `<body>`, and with it the page-level `style`,
/// `align` and `bgcolor` that many layouts rely on for centering. Keep them on
/// a `div` that goes through the same attribute sanitization as any other.
fn body_as_div(value: &str) -> String {
    let lower = value.to_ascii_lowercase();
    let mut out = String::with_capacity(value.len());
    let mut at = 0;
    while let Some(found) = lower[at..].find("<body") {
        let start = at + found;
        let after = start + "<body".len();
        if !matches!(
            lower.as_bytes().get(after),
            Some(b' ' | b'\t' | b'\r' | b'\n' | b'>' | b'/')
        ) {
            out.push_str(&value[at..after]);
            at = after;
            continue;
        }
        let mut quote = None;
        let mut end = None;
        for (offset, byte) in value.bytes().enumerate().skip(after) {
            match (quote, byte) {
                (Some(q), b) if b == q => quote = None,
                (None, b'"' | b'\'') => quote = Some(byte),
                (None, b'>') => {
                    end = Some(offset);
                    break;
                }
                _ => {}
            }
        }
        let Some(end) = end else { break };
        out.push_str(&value[at..start]);
        out.push_str("<div data-mailx-body");
        out.push_str(&value[after..end]);
        out.push('>');
        at = end + 1;
    }
    out.push_str(&value[at..]);
    out.replace("</body>", "</div>")
        .replace("</BODY>", "</div>")
}

pub(crate) fn sanitize(value: &str) -> Sanitized {
    let value = body_as_div(value);
    let mut document = Html::parse_fragment(&value);
    let mut links = Vec::new();
    let mut data_images = Vec::new();
    let mut blocked = 0u32;
    // Remove entire active/foreign subtrees before creating any trusted slots.
    // Pre-existing slot attributes are removed from every retained element below.
    let ids: Vec<_> = document
        .tree
        .root()
        .descendants()
        .map(|node| node.id())
        .collect();
    for id in ids {
        let node = document.tree.get(id).unwrap();
        let remove = match node.value() {
            Node::Element(element) => {
                element.name.ns.as_ref() != "http://www.w3.org/1999/xhtml"
                    || matches!(
                        element.name(),
                        "script"
                            | "iframe"
                            | "object"
                            | "embed"
                            | "form"
                            | "input"
                            | "button"
                            | "meta"
                            | "title"
                            | "base"
                            | "link"
                            | "svg"
                            | "math"
                            | "template"
                            | "noscript"
                            | "textarea"
                            | "select"
                            | "audio"
                            | "video"
                    )
            }
            Node::Comment(_) | Node::ProcessingInstruction(_) => true,
            _ => false,
        };
        if remove {
            // Posters and image inputs are still image references even though
            // their active containing elements will not survive.
            if let Node::Element(element) = node.value() {
                for (name, value) in &element.attrs {
                    if (name.local.as_ref() == "poster"
                        || (element.name() == "input"
                            && element.attr("type") == Some("image")
                            && name.local.as_ref() == "src"))
                        && is_remote(value)
                    {
                        blocked = blocked.saturating_add(1);
                    }
                }
            }
            document.tree.get_mut(id).unwrap().detach();
        }
    }
    let ids: Vec<_> = document
        .tree
        .root()
        .descendants()
        .map(|node| node.id())
        .collect();
    for id in ids {
        let style_content = {
            let node = document.tree.get(id).unwrap();
            if matches!(node.value(), Node::Element(element) if element.name() == "style") {
                Some(
                    node.children()
                        .filter_map(|node| node.value().as_text())
                        .map(|text| text.text.as_ref())
                        .collect::<String>(),
                )
            } else {
                None
            }
        };
        if let Some(content) = style_content {
            let (clean, count) = css::stylesheet(&content);
            blocked = blocked.saturating_add(count);
            let children: Vec<_> = document
                .tree
                .get(id)
                .unwrap()
                .children()
                .map(|node| node.id())
                .collect();
            for child in children {
                document.tree.get_mut(child).unwrap().detach();
            }
            document
                .tree
                .get_mut(id)
                .unwrap()
                .append(Node::Text(Text { text: clean.into() }));
        }
        let mut node = document.tree.get_mut(id).unwrap();
        let Node::Element(element) = node.value() else {
            continue;
        };
        let tag = element.name().to_owned();
        let attrs = std::mem::take(&mut element.attrs);
        let mut is_body = false;
        for (name, value) in attrs {
            let attr = name.local.as_ref();
            if !name.ns.is_empty() || attr.starts_with("data-mailx-") {
                is_body |= name.local.as_ref() == BODY_MARK;
                continue;
            }
            match attr {
                "style" => {
                    let (clean, count) = css::declarations(&value);
                    blocked = blocked.saturating_add(count);
                    if !clean.is_empty() {
                        set_attr(element, "style", &clean);
                    }
                }
                "srcset" => blocked = blocked.saturating_add(srcset_remote_count(&value)),
                "background" | "poster" => {
                    if is_remote(&value) {
                        blocked = blocked.saturating_add(1);
                    }
                }
                "src" if tag == "img" => {
                    if let Some(url) = remote_url(&value) {
                        // Keep a provenance-marked inert URL; never an external src.
                        if blocked < MAX_REMOTE_IMAGES as u32 {
                            set_attr(element, REMOTE_SLOT, &url);
                        }
                        blocked = blocked.saturating_add(1);
                    } else if value
                        .get(..4)
                        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("cid:"))
                    {
                        if let Some(cid) = images::percent_decode(&value[4..])
                            .and_then(|bytes| String::from_utf8(bytes).ok())
                        {
                            let cid = super::normalize_cid(&cid);
                            if !cid.is_empty() && !cid.chars().any(char::is_control) {
                                set_attr(element, CID_SLOT, &cid);
                            }
                        }
                    } else if let Some(data) = images::data_url(&value) {
                        set_attr(element, DATA_SLOT, &data_images.len().to_string());
                        data_images.push(data);
                    }
                }
                "href" if tag == "a" => {
                    if let Some(link) = canonical_link(&value) {
                        set_attr(element, LINK_SLOT, &links.len().to_string());
                        set_attr(element, "title", &link);
                        links.push(link);
                    }
                }
                // Only the branches above can authorize a URL. Strip legacy,
                // media and navigation URL attributes regardless of tag.
                "href" | "src" | "action" | "formaction" | "ping" | "data" | "cite"
                | "longdesc" | "usemap" | "profile" | "manifest" | "codebase" | "archive"
                | "lowsrc" | "dynsrc" => {}
                _ => element.attrs.push((name, value)),
            }
        }
        if is_body {
            let class = element
                .attr("class")
                .map_or("mailx-body".to_owned(), |class| {
                    format!("mailx-body {class}")
                });
            set_attr(element, "class", &class);
        }
        if tag == "a"
            && let Some(link) = element
                .attr(LINK_SLOT)
                .and_then(|v| v.parse::<usize>().ok())
                .and_then(|i| links.get(i))
        {
            set_attr(element, "title", link);
        }
    }
    let mut builder = ammonia::Builder::default();
    builder
        .url_schemes(
            ["http", "https", "mailto", "cid"]
                .into_iter()
                .collect::<HashSet<_>>(),
        )
        .url_relative(ammonia::UrlRelative::Deny)
        .add_tags(&[
            "style", "font", "big", "tfoot", "section", "main", "address", "nobr", "dir", "menu",
            "label",
        ])
        .rm_clean_content_tags(&["style"])
        .add_generic_attributes(&[
            "style",
            "class",
            "align",
            "valign",
            "bgcolor",
            "width",
            "height",
            "border",
            "cellpadding",
            "cellspacing",
            "nowrap",
            "hspace",
            "vspace",
            "bordercolor",
            "dir",
            "id",
            "role",
        ])
        .add_generic_attribute_prefixes(&["aria-"])
        .add_tag_attributes("font", &["color", "size", "face"])
        .add_tag_attributes("ol", &["type", "reversed"])
        .add_tag_attributes("ul", &["type"])
        .add_tag_attributes("li", &["type", "value"])
        .add_tag_attributes("table", &["frame", "rules"])
        .add_tag_attributes("hr", &["color", "noshade"])
        .add_tag_attributes("br", &["clear"])
        .add_tag_attributes("img", &[CID_SLOT, DATA_SLOT, REMOTE_SLOT])
        .add_tag_attributes("a", &[LINK_SLOT])
        // Apple Mail and Thunderbird mark quotes with `blockquote type="cite"`.
        .add_tag_attributes("blockquote", &["type"])
        .link_rel(Some("noopener noreferrer"))
        .strip_comments(true);
    let cleaned = builder
        .clean(&document.root_element().inner_html())
        .to_string();
    let mut document = Html::parse_fragment(&cleaned);
    // Validated data: images pass through a provenance slot, never a globally
    // permitted data: scheme. Attacker-supplied slot attributes were removed.
    let ids: Vec<_> = document
        .tree
        .root()
        .descendants()
        .map(|node| node.id())
        .collect();
    for id in ids {
        let mut node = document.tree.get_mut(id).unwrap();
        if let Node::Element(element) = node.value() {
            let data = (element.name() == "img")
                .then(|| element.attr(DATA_SLOT))
                .flatten()
                .and_then(|value| value.parse::<usize>().ok())
                .and_then(|index| data_images.get(index))
                .cloned();
            element
                .attrs
                .retain(|(name, _)| name.local.as_ref() != DATA_SLOT);
            if let Some(data) = data {
                set_attr(element, "src", &data);
            }
        }
    }
    Sanitized {
        html: document.root_element().inner_html(),
        links,
        remote_images_blocked: blocked,
    }
}

fn set_attr(element: &mut scraper::node::Element, name: &str, value: &str) {
    element.attrs.retain(|(key, _)| key.local.as_ref() != name);
    element
        .attrs
        .push((QualName::new(None, "".into(), name.into()), value.into()));
}

fn srcset_remote_count(value: &str) -> u32 {
    let mut remaining = value;
    let mut count = 0u32;
    while !remaining.is_empty() {
        remaining = remaining.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ',');
        let end = remaining
            .find(char::is_whitespace)
            .unwrap_or(remaining.len());
        let candidate = &remaining[..end];
        remaining = &remaining[end..];
        if is_remote(candidate.trim_end_matches(',')) {
            count = count.saturating_add(1);
        }
        if !candidate.ends_with(',') {
            remaining = remaining.split_once(',').map_or("", |(_, tail)| tail);
        }
    }
    count
}

/// Flatten sanitized HTML to readable text: block elements and `<br>` become
/// line breaks, list items get a `- ` prefix (indented two spaces per nesting
/// level), table cells are separated by a space, `<pre>` keeps its line breaks,
/// and every other run of whitespace collapses to one space. Lines are trimmed
/// and blank-line runs shrink to a single blank line. Style, script, head and
/// title content is not text of the message.
pub(super) fn plain_text(html: &str) -> String {
    enum Visit<'a> {
        Enter(ego_tree::NodeRef<'a, Node>),
        Exit(&'a str),
    }

    let document = Html::parse_fragment(html);
    let mut out = TextOut::default();
    let mut stack = vec![Visit::Enter(document.tree.root())];
    while let Some(visit) = stack.pop() {
        match visit {
            Visit::Enter(node) => match node.value() {
                Node::Text(text) => out.text(text.text.as_ref()),
                Node::Element(element) => {
                    let name = element.name();
                    if matches!(name, "style" | "script" | "head" | "title" | "template") {
                        continue;
                    }
                    out.enter(name);
                    stack.push(Visit::Exit(name));
                    let children: Vec<_> = node.children().collect();
                    stack.extend(children.into_iter().rev().map(Visit::Enter));
                }
                _ => {
                    let children: Vec<_> = node.children().collect();
                    stack.extend(children.into_iter().rev().map(Visit::Enter));
                }
            },
            Visit::Exit(name) => out.exit(name),
        }
    }
    out.finish()
}

#[derive(Default)]
struct TextOut {
    buf: String,
    /// Line breaks owed before the next text (1 = new line, 2 = blank line).
    pending: u8,
    /// A collapsed inline space is owed before the next word.
    space: bool,
    prefix: Option<String>,
    /// Breaks owed when the current list item opened; nested blocks cannot add more.
    prefix_cap: u8,
    list_depth: usize,
    pre_depth: usize,
}

impl TextOut {
    fn breaks(&mut self, count: u8) {
        self.pending = self.pending.max(count);
        self.space = false;
    }

    fn enter(&mut self, name: &str) {
        match name {
            "br" => {
                self.pending = (self.pending + 1).min(2);
                self.space = false;
            }
            "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "blockquote" | "hr" => self.breaks(2),
            "pre" => {
                self.breaks(2);
                self.pre_depth += 1;
            }
            "ul" | "ol" | "menu" => {
                self.breaks(1);
                self.list_depth += 1;
            }
            "li" => {
                self.breaks(1);
                self.prefix_cap = self.pending;
                self.prefix = Some(format!(
                    "{}- ",
                    "  ".repeat(self.list_depth.saturating_sub(1))
                ));
            }
            "td" | "th" => self.space = true,
            "div" | "tr" | "table" | "section" | "article" | "header" | "footer" | "nav"
            | "aside" | "main" | "address" | "figure" | "figcaption" | "dl" | "dt" | "dd"
            | "fieldset" | "form" | "details" | "summary" | "caption" | "thead" | "tbody"
            | "tfoot" | "center" => self.breaks(1),
            _ => {}
        }
    }

    fn exit(&mut self, name: &str) {
        match name {
            "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "blockquote" => self.breaks(2),
            "pre" => {
                self.pre_depth = self.pre_depth.saturating_sub(1);
                self.breaks(2);
            }
            "ul" | "ol" | "menu" => {
                self.list_depth = self.list_depth.saturating_sub(1);
                self.breaks(1);
            }
            "td" | "th" => self.space = true,
            "li" | "div" | "tr" | "table" | "section" | "article" | "header" | "footer" | "nav"
            | "aside" | "main" | "address" | "figure" | "figcaption" | "dl" | "dt" | "dd"
            | "fieldset" | "form" | "details" | "summary" | "caption" | "thead" | "tbody"
            | "tfoot" | "center" => self.breaks(1),
            _ => {}
        }
    }

    /// Write owed breaks, list prefix and inline space ahead of visible text.
    fn start_text(&mut self) {
        if self.prefix.is_some() {
            self.pending = self.pending.min(self.prefix_cap);
        }
        if !self.buf.is_empty() {
            for _ in 0..self.pending {
                self.buf.push('\n');
            }
        }
        let line_start = self.buf.is_empty() || self.buf.ends_with('\n');
        self.pending = 0;
        if let Some(prefix) = self.prefix.take() {
            self.buf.push_str(&prefix);
        } else if self.space && !line_start {
            self.buf.push(' ');
        }
        self.space = false;
    }

    fn text(&mut self, text: &str) {
        if self.pre_depth > 0 {
            if text.is_empty() {
                return;
            }
            self.start_text();
            let mut lines = text.split('\n').peekable();
            while let Some(line) = lines.next() {
                self.buf.push_str(line.trim_end_matches('\r'));
                if lines.peek().is_some() {
                    self.buf.push('\n');
                }
            }
            return;
        }
        if text.starts_with(char::is_whitespace) {
            self.space = true;
        }
        for (index, word) in text.split_whitespace().enumerate() {
            if index > 0 {
                self.space = true;
            }
            self.start_text();
            self.buf.push_str(word);
        }
        if text.ends_with(char::is_whitespace) {
            self.space = true;
        }
    }

    fn finish(self) -> String {
        let mut result = String::with_capacity(self.buf.len());
        let mut blank = false;
        for line in self.buf.split('\n') {
            let line = line.trim_end();
            if line.is_empty() {
                blank = !result.is_empty();
                continue;
            }
            if !result.is_empty() {
                result.push('\n');
                if blank {
                    result.push('\n');
                }
            }
            blank = false;
            result.push_str(line);
        }
        result
    }
}
