use std::collections::HashSet;

use html5ever::QualName;
use scraper::{Html, Node, node::Text};
use url::Url;

use super::{css, images};

const CID_SLOT: &str = "data-mailx-cid";
const LINK_SLOT: &str = "data-mailx-link";
const DATA_SLOT: &str = "data-mailx-data";

pub(super) struct Sanitized {
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
}

/// Bind a trusted renderer result at serving time. Unresolved slots are removed.
/// Host-generated URLs must use one of Tauri's two supported protocol origins and
/// the matching resource route. Email text is never inspected or substituted.
pub fn bind_resources(
    sanitized_html: &str,
    mut resolve: impl FnMut(ResourceBinding<'_>) -> Option<String>,
) -> String {
    let mut document = Html::parse_fragment(sanitized_html);
    let ids: Vec<_> = document
        .tree
        .root()
        .descendants()
        .map(|node| node.id())
        .collect();
    for id in ids {
        let mut node = document.tree.get_mut(id).unwrap();
        let Node::Element(element) = node.value() else {
            continue;
        };
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
        let Some((value, part)) = slot else { continue };
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
            .starts_with(if part { "/part/" } else { "/link/" })
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

pub(super) fn sanitize(value: &str) -> Sanitized {
    let mut document = Html::parse_fragment(value);
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
        for (name, value) in attrs {
            let attr = name.local.as_ref();
            if !name.ns.is_empty() || attr.starts_with("data-mailx-") {
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
                    if is_remote(&value) {
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
    }
    let mut builder = ammonia::Builder::default();
    builder
        .url_schemes(
            ["http", "https", "mailto", "cid"]
                .into_iter()
                .collect::<HashSet<_>>(),
        )
        .url_relative(ammonia::UrlRelative::Deny)
        .add_tags(&["style"])
        .rm_clean_content_tags(&["style"])
        .add_generic_attributes(&["style", "class"])
        .add_tag_attributes("img", &[CID_SLOT, DATA_SLOT])
        .add_tag_attributes("a", &[LINK_SLOT])
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

pub(super) fn plain_text(html: &str) -> String {
    let document = Html::parse_fragment(html);
    document.tree.root().descendants().filter_map(|node| {
        let text = node.value().as_text()?;
        if node.parent().is_some_and(|parent| matches!(parent.value(), Node::Element(element) if element.name() == "style")) {
            return None;
        }
        Some(text.text.as_ref())
    }).flat_map(str::split_whitespace).collect::<Vec<_>>().join(" ")
}
