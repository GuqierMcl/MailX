//! The text a translation service sees: a segment as a small HTML fragment
//! whose only markup is `span`, `br` and `img` carrying a numeric `id`.
//! A second, tag-name form (`<g1>..</g1>`, `<n1/>`, `<x1/>`) exists for a
//! service that mangles attributes; it is only ever a transport encoding, and
//! its answers are turned back into pieces and then into the HTML form here.
//! Decoding is deliberately lenient and yields nothing but pieces; whether a
//! decoded answer fits its source segment is checked by the applier.
use ego_tree::iter::Edge;
use scraper::{Html, Node, node::Element};

use super::{Piece, Segment};

pub fn encode(segment: &Segment) -> String {
    encode_pieces(segment.pieces())
}

/// The HTML form of a piece list; the only form that is stored and applied.
pub fn encode_pieces(pieces: &[Piece]) -> String {
    let mut out = String::new();
    for piece in pieces {
        match piece {
            Piece::Text(text) => escape(text, &mut out),
            Piece::Open(n) => out.push_str(&format!(r#"<span id="{n}">"#)),
            Piece::Close(_) => out.push_str("</span>"),
            Piece::Break(n) => out.push_str(&format!(r#"<br id="{n}">"#)),
            Piece::Object(n) => out.push_str(&format!(r#"<img id="{n}">"#)),
        }
    }
    out
}

/// Every close matches the innermost open placeholder and none stays open.
pub fn tags_balanced(pieces: &[Piece]) -> bool {
    let mut open: Vec<u16> = Vec::new();
    for piece in pieces {
        match piece {
            Piece::Open(n) => open.push(*n),
            Piece::Close(n) if open.last() == Some(n) => {
                open.pop();
            }
            Piece::Close(_) => return false,
            _ => {}
        }
    }
    open.is_empty()
}

fn escape(text: &str, out: &mut String) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
}

/// `span`, `br` or `img` with a purely numeric `id` that fits a placeholder.
fn placeholder(element: &Element) -> Option<(&str, u16)> {
    let name = element.name();
    if !matches!(name, "span" | "br" | "img") {
        return None;
    }
    let id = element.attr("id")?;
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((name, id.parse().ok()?))
}

pub fn decode(text: &str) -> Vec<Piece> {
    let fragment = Html::parse_fragment(text);
    let mut out: Vec<Piece> = Vec::new();
    for edge in fragment.root_element().traverse() {
        match edge {
            Edge::Open(node) => match node.value() {
                Node::Text(text) if !text.text.is_empty() => match out.last_mut() {
                    Some(Piece::Text(last)) => last.push_str(&text.text),
                    _ => out.push(Piece::Text(text.text.to_string())),
                },
                Node::Element(element) => match placeholder(element) {
                    Some(("span", n)) => out.push(Piece::Open(n)),
                    Some(("br", n)) => out.push(Piece::Break(n)),
                    Some((_, n)) => out.push(Piece::Object(n)),
                    // Any other element contributes only its children.
                    None => {}
                },
                _ => {}
            },
            Edge::Close(node) => {
                if let Node::Element(element) = node.value()
                    && let Some(("span", n)) = placeholder(element)
                {
                    out.push(Piece::Close(n));
                }
            }
        }
    }
    out
}

/// Tag-name encoding: `<gN>` / `</gN>` wrap an element, `<nN/>` is a break and
/// `<xN/>` an object. Text escapes only `&`, `<` and `>`, so the service sees a
/// literal `<g1>` typed by the sender as `&lt;g1&gt;`.
pub fn encode_tags(pieces: &[Piece]) -> String {
    let mut out = String::new();
    for piece in pieces {
        match piece {
            Piece::Text(text) => {
                for ch in text.chars() {
                    match ch {
                        '&' => out.push_str("&amp;"),
                        '<' => out.push_str("&lt;"),
                        '>' => out.push_str("&gt;"),
                        _ => out.push(ch),
                    }
                }
            }
            Piece::Open(n) => out.push_str(&format!("<g{n}>")),
            Piece::Close(n) => out.push_str(&format!("</g{n}>")),
            Piece::Break(n) => out.push_str(&format!("<n{n}/>")),
            Piece::Object(n) => out.push_str(&format!("<x{n}/>")),
        }
    }
    out
}

/// Entities a service may hand back; any other `&` stays literal text.
const ENTITIES: [(&str, char); 6] = [
    ("&amp;", '&'),
    ("&lt;", '<'),
    ("&gt;", '>'),
    ("&quot;", '"'),
    ("&#39;", '\''),
    ("&apos;", '\''),
];

/// One exact placeholder token at the start of `rest` (which begins with `<`):
/// the piece and its length in bytes.
fn tag_token(rest: &str) -> Option<(Piece, usize)> {
    let bytes = rest.as_bytes();
    let closing = bytes.get(1) == Some(&b'/');
    let kind = *bytes.get(if closing { 2 } else { 1 })?;
    if !matches!(kind, b'g' | b'n' | b'x') || (closing && kind != b'g') {
        return None;
    }
    let digits_at = if closing { 3 } else { 2 };
    let digits = bytes[digits_at..]
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .count();
    if !(1..=4).contains(&digits) {
        return None;
    }
    let number: u16 = rest[digits_at..digits_at + digits].parse().ok()?;
    let tail = &rest[digits_at + digits..];
    let (piece, tail_len) = match kind {
        b'g' if tail.starts_with('>') => (
            if closing {
                Piece::Close(number)
            } else {
                Piece::Open(number)
            },
            1,
        ),
        b'n' | b'x' if !closing => {
            // Some services put a space before `/>`.
            let tail_len = if tail.starts_with("/>") {
                2
            } else if tail.starts_with(" />") {
                3
            } else {
                return None;
            };
            (
                if kind == b'n' {
                    Piece::Break(number)
                } else {
                    Piece::Object(number)
                },
                tail_len,
            )
        }
        _ => return None,
    };
    Some((piece, digits_at + digits + tail_len))
}

/// A strict tokenizer, not an HTML parser: only the exact tokens that
/// `encode_tags` writes become placeholders, every other character is text,
/// and only the entities in `ENTITIES` are decoded (once).
pub fn decode_tags(text: &str) -> Vec<Piece> {
    fn push_text(out: &mut Vec<Piece>, ch: char) {
        match out.last_mut() {
            Some(Piece::Text(last)) => last.push(ch),
            _ => out.push(Piece::Text(ch.to_string())),
        }
    }
    let mut out: Vec<Piece> = Vec::new();
    let mut at = 0;
    while at < text.len() {
        let rest = &text[at..];
        if rest.starts_with('<')
            && let Some((piece, len)) = tag_token(rest)
        {
            out.push(piece);
            at += len;
            continue;
        }
        if rest.starts_with('&')
            && let Some((entity, ch)) = ENTITIES.iter().find(|(entity, _)| rest.starts_with(entity))
        {
            push_text(&mut out, *ch);
            at += entity.len();
            continue;
        }
        let Some(ch) = rest.chars().next() else {
            break;
        };
        push_text(&mut out, ch);
        at += ch.len_utf8();
    }
    out
}
