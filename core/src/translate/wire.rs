//! The text a translation service sees: a segment as a small HTML fragment
//! whose only markup is `span`, `br` and `img` carrying a numeric `id`.
//! Decoding is deliberately lenient and yields nothing but pieces; whether a
//! decoded answer fits its source segment is checked by the applier.
use ego_tree::iter::Edge;
use scraper::{Html, Node, node::Element};

use super::{Piece, Segment};

pub fn encode(segment: &Segment) -> String {
    let mut out = String::new();
    for piece in segment.pieces() {
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
