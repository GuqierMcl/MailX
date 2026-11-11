//! The single document walk behind `extract` and `apply`.
//!
//! It is iterative (hostile nesting must not overflow the stack) and computes
//! per-element facts in one bottom-up pass, so deep inline nesting stays linear.
//! Segment order is document order; both entry points run `prepare` and `plan`
//! on the same HTML, so their segment lists are identical by construction.
use std::collections::{HashMap, HashSet};

use ego_tree::{NodeId, NodeRef, Tree, iter::Edge};
use html5ever::QualName;
use scraper::{
    Html, Node,
    node::{Element, Text},
};

use super::{MAX_SEGMENTS, Piece, Segment};

type Ref<'a> = NodeRef<'a, Node>;

/// A segment with more placeholders than this is not worth a model's attention.
const MAX_PLACEHOLDERS: usize = 64;
/// A line this long, followed by text, was wrapped by the sender.
const SOFT_WRAP_MIN: usize = 60;

const SKIPPED: [&str; 9] = [
    "style", "script", "title", "head", "svg", "math", "select", "textarea", "template",
];
const INLINE: [&str; 25] = [
    "a", "abbr", "b", "bdi", "bdo", "big", "cite", "del", "dfn", "em", "font", "i", "ins", "mark",
    "q", "s", "small", "span", "strike", "strong", "sub", "sup", "time", "u", "var",
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum SlotKind {
    Open,
    Break,
    Object,
}

pub(super) struct Slot {
    pub kind: SlotKind,
    pub node: NodeId,
    /// Losing it would lose a link or an image, so a translation must keep it.
    pub required: bool,
}

pub(super) struct Planned {
    /// Consecutive siblings that make up the segment.
    pub nodes: Vec<NodeId>,
    /// Placeholder `n` is `slots[n - 1]`.
    pub slots: Vec<Slot>,
    pub segment: Segment,
    /// The original text had whitespace at the edge; one space is restored.
    pub lead: bool,
    pub trail: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Never segmented and never entered.
    Skip,
    Block,
    /// Inline element that must be entered like a block.
    Split,
    Br,
    /// Opaque: kept as is, content never sent.
    Object,
    Inline,
}

struct Info {
    kind: Kind,
    /// Non-whitespace text below, not counting opaque `code` and friends.
    text: bool,
}

pub(super) fn qual(name: &str) -> QualName {
    QualName::new(
        None,
        "http://www.w3.org/1999/xhtml".into(),
        name.to_owned().into(),
    )
}

fn is_inline(name: &str) -> bool {
    INLINE.contains(&name)
}

fn is_quote(element: &Element) -> bool {
    element.name() == "details" && element.classes().any(|class| class == "mailx-quote")
}

fn is_skipped(node: Ref, element: &Element, include_quotes: bool) -> bool {
    let name = element.name();
    if SKIPPED.contains(&name) || element.attr("hidden").is_some() {
        return true;
    }
    if let Some(style) = element.attr("style") {
        let compact: String = style
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect::<String>()
            .to_lowercase();
        if compact.contains("display:none") {
            return true;
        }
    }
    if is_quote(element) {
        return !include_quotes;
    }
    // The folded-quote label is host text, never part of the message.
    name == "summary"
        && node
            .parent()
            .and_then(|parent| parent.value().as_element())
            .is_some_and(is_quote)
}

// ------------------------------------------------------------------ prepare

/// `Some(true)` when the element's own style or name makes newlines
/// meaningful, `Some(false)` when its style restores normal wrapping.
fn pre_override(element: &Element) -> Option<bool> {
    let mut found = None;
    for declaration in element.attr("style").unwrap_or_default().split(';') {
        let Some((name, value)) = declaration.split_once(':') else {
            continue;
        };
        if name.trim().eq_ignore_ascii_case("white-space") {
            let value = value.trim().to_ascii_lowercase();
            let value = value.trim_end_matches("!important").trim();
            found = Some(matches!(
                value,
                "pre" | "pre-wrap" | "pre-line" | "break-spaces"
            ));
        }
    }
    found.or((element.name() == "pre").then_some(true))
}

enum Chunk {
    Text(String),
    Break,
}

/// Soft newline (the line before it is long and a line follows): the `\n` stays
/// in the text, so untranslated text renders as before; segment text collapses
/// it to a space. Every other newline becomes a real `<br>`.
fn split_lines(text: &str, start: usize, next_filled: bool) -> Vec<Chunk> {
    let parts: Vec<&str> = text.split('\n').collect();
    let mut out = Vec::new();
    let mut current = String::new();
    for (index, part) in parts.iter().enumerate() {
        current.push_str(part);
        let Some(next) = parts.get(index + 1) else {
            break;
        };
        let before = part.chars().count() + if index == 0 { start } else { 0 };
        let filled = !next.is_empty() || (index + 2 == parts.len() && next_filled);
        if before >= SOFT_WRAP_MIN && filled {
            current.push('\n');
        } else {
            out.push(Chunk::Text(std::mem::take(&mut current)));
            out.push(Chunk::Break);
        }
    }
    out.push(Chunk::Text(current));
    out
}

/// Make newlines in pre-formatted text structural, in the parsed tree, so the
/// generic rules apply. Skipped subtrees are never touched.
pub(super) fn prepare(document: &mut Html, include_quotes: bool) {
    let mut inert: HashSet<NodeId> = HashSet::new();
    let mut pre: HashSet<NodeId> = HashSet::new();
    let mut work: Vec<(NodeId, usize)> = Vec::new();
    // Characters on the current line, carried across inline elements.
    let mut line = 0usize;
    for node in document.root_element().descendants() {
        let parent = node.parent().map(|parent| parent.id());
        let within = |set: &HashSet<NodeId>| parent.is_some_and(|id| set.contains(&id));
        match node.value() {
            Node::Element(element) => {
                if within(&inert) || is_skipped(node, element, include_quotes) {
                    inert.insert(node.id());
                    continue;
                }
                if pre_override(element).unwrap_or_else(|| within(&pre)) {
                    pre.insert(node.id());
                }
                if element.name() == "br" || !is_inline(element.name()) {
                    line = 0;
                }
            }
            Node::Text(text) => {
                if within(&inert) || !within(&pre) {
                    continue;
                }
                work.push((node.id(), line));
                line = match text.text.rfind('\n') {
                    Some(at) => text.text[at + 1..].chars().count(),
                    None => line + text.text.chars().count(),
                };
            }
            _ => {}
        }
    }
    let tree = &mut document.tree;
    for (id, start) in work {
        let node = tree.get(id).unwrap();
        let text = match node.value() {
            Node::Text(text) => text.text.to_string(),
            _ => continue,
        };
        if !text.contains('\n') {
            continue;
        }
        let next_filled = match node.next_sibling().map(|next| next.value()) {
            Some(Node::Text(next)) => !next.text.is_empty(),
            Some(Node::Element(next)) => is_inline(next.name()),
            _ => false,
        };
        let mut chunks = split_lines(&text, start, next_filled).into_iter();
        let Some(Chunk::Text(first)) = chunks.next() else {
            continue;
        };
        if let Node::Text(node_text) = tree.get_mut(id).unwrap().value() {
            node_text.text = first.into();
        }
        let mut last = id;
        for chunk in chunks {
            let value = match chunk {
                Chunk::Text(text) if text.is_empty() => continue,
                Chunk::Text(text) => Node::Text(Text { text: text.into() }),
                Chunk::Break => Node::Element(Element::new(qual("br"), vec![])),
            };
            let new = tree.orphan(value).id();
            tree.get_mut(last).unwrap().insert_id_after(new);
            last = new;
        }
    }
}

// ----------------------------------------------------------------- classify

/// One bottom-up pass: children are visited before their parents because the
/// pre-order list is walked backwards.
fn classify(document: &Html, include_quotes: bool) -> HashMap<NodeId, Info> {
    let order: Vec<NodeId> = document
        .root_element()
        .descendants()
        .map(|node| node.id())
        .collect();
    let mut infos: HashMap<NodeId, Info> = HashMap::new();
    for id in order.into_iter().rev() {
        let node = document.tree.get(id).unwrap();
        let Some(element) = node.value().as_element() else {
            continue;
        };
        let mut text = false;
        // Something below forces the element to be entered, not flattened.
        let mut splits = false;
        let mut after_break = false;
        for child in node.children() {
            match child.value() {
                Node::Text(value) => {
                    if value.text.chars().any(|ch| !ch.is_whitespace()) {
                        text = true;
                        after_break = false;
                    }
                }
                Node::Element(_) => {
                    let info = &infos[&child.id()];
                    match info.kind {
                        Kind::Skip | Kind::Block | Kind::Split => {
                            splits = true;
                            after_break = false;
                        }
                        Kind::Br => {
                            // Two breaks in a row, whitespace aside.
                            splits |= after_break;
                            after_break = true;
                        }
                        Kind::Object => after_break = false,
                        Kind::Inline => {
                            text |= info.text;
                            after_break = false;
                        }
                    }
                }
                _ => after_break = false,
            }
        }
        let name = element.name();
        let kind = if is_skipped(node, element, include_quotes) {
            Kind::Skip
        } else if name == "br" {
            Kind::Br
        } else if matches!(name, "img" | "wbr" | "code" | "kbd" | "samp") {
            Kind::Object
        } else if is_inline(name) {
            if splits {
                Kind::Split
            } else if !text {
                Kind::Object
            } else {
                Kind::Inline
            }
        } else {
            Kind::Block
        };
        infos.insert(
            id,
            Info {
                kind,
                text: kind == Kind::Inline && text,
            },
        );
    }
    infos
}

// --------------------------------------------------------------------- plan

enum Item {
    /// Children of this element are processed in order.
    Container(NodeId),
    /// Consecutive nodes that belong to one flow of text.
    Run(Vec<NodeId>),
    /// A run split at paragraph breaks and trimmed.
    Candidate(Vec<NodeId>),
}

pub(super) fn plan(document: &Html, include_quotes: bool) -> Vec<Planned> {
    let infos = classify(document, include_quotes);
    let tree = &document.tree;
    let mut out: Vec<Planned> = Vec::new();
    let mut stack = vec![Item::Container(document.root_element().id())];
    while let Some(item) = stack.pop() {
        if out.len() >= MAX_SEGMENTS {
            break;
        }
        match item {
            Item::Container(id) => {
                let items = split_container(tree, &infos, id);
                stack.extend(items.into_iter().rev());
            }
            Item::Run(nodes) => {
                let candidates = split_run(tree, &infos, nodes);
                stack.extend(candidates.into_iter().rev().map(Item::Candidate));
            }
            Item::Candidate(nodes) => {
                // `<p><span><font>text</font></span></p>`: enter the wrapper.
                if let [only] = nodes.as_slice()
                    && infos
                        .get(only)
                        .is_some_and(|info| info.kind == Kind::Inline)
                {
                    stack.push(Item::Container(*only));
                } else if let Some(planned) = build(tree, &infos, nodes) {
                    out.push(planned);
                }
            }
        }
    }
    out
}

fn kind_of(infos: &HashMap<NodeId, Info>, node: Ref) -> Option<Kind> {
    infos.get(&node.id()).map(|info| info.kind)
}

fn split_container(tree: &Tree<Node>, infos: &HashMap<NodeId, Info>, id: NodeId) -> Vec<Item> {
    let mut items = Vec::new();
    let mut run: Vec<NodeId> = Vec::new();
    let flush = |run: &mut Vec<NodeId>, items: &mut Vec<Item>| {
        if !run.is_empty() {
            items.push(Item::Run(std::mem::take(run)));
        }
    };
    for child in tree.get(id).unwrap().children() {
        match child.value() {
            Node::Text(_) => run.push(child.id()),
            Node::Element(_) => match kind_of(infos, child) {
                Some(Kind::Br | Kind::Object | Kind::Inline) => run.push(child.id()),
                Some(Kind::Block | Kind::Split) => {
                    flush(&mut run, &mut items);
                    items.push(Item::Container(child.id()));
                }
                _ => flush(&mut run, &mut items),
            },
            _ => flush(&mut run, &mut items),
        }
    }
    flush(&mut run, &mut items);
    items
}

fn is_blank_text(node: Ref) -> bool {
    node.value()
        .as_text()
        .is_some_and(|text| text.text.chars().all(char::is_whitespace))
}

fn is_break(infos: &HashMap<NodeId, Info>, node: Ref) -> bool {
    kind_of(infos, node) == Some(Kind::Br)
}

/// Split at paragraph breaks (two or more `br`, whitespace between ignored),
/// then trim blank text and breaks from both ends. Trimmed nodes stay in the
/// document as they were.
fn split_run(
    tree: &Tree<Node>,
    infos: &HashMap<NodeId, Info>,
    nodes: Vec<NodeId>,
) -> Vec<Vec<NodeId>> {
    let refs: Vec<Ref> = nodes.iter().map(|id| tree.get(*id).unwrap()).collect();
    let mut pieces: Vec<Vec<NodeId>> = vec![Vec::new()];
    let mut at = 0;
    while at < refs.len() {
        if !is_break(infos, refs[at]) {
            pieces.last_mut().unwrap().push(refs[at].id());
            at += 1;
            continue;
        }
        let mut end = at;
        let mut breaks = 0;
        while end < refs.len() && (is_break(infos, refs[end]) || is_blank_text(refs[end])) {
            breaks += usize::from(is_break(infos, refs[end]));
            end += 1;
        }
        if breaks >= 2 {
            pieces.push(Vec::new());
        } else {
            pieces
                .last_mut()
                .unwrap()
                .extend(refs[at..end].iter().map(|node| node.id()));
        }
        at = end;
    }
    pieces
        .into_iter()
        .filter_map(|mut piece| {
            let trimmable = |id: &NodeId| {
                let node = tree.get(*id).unwrap();
                is_blank_text(node) || is_break(infos, node)
            };
            let start = piece.iter().position(|id| !trimmable(id))?;
            let end = piece.iter().rposition(|id| !trimmable(id))?;
            piece.truncate(end + 1);
            piece.drain(..start);
            Some(piece)
        })
        .collect()
}

fn push_text(pieces: &mut Vec<Piece>, text: &str) {
    for ch in text.chars() {
        if !matches!(pieces.last(), Some(Piece::Text(_))) {
            pieces.push(Piece::Text(String::new()));
        }
        let Some(Piece::Text(last)) = pieces.last_mut() else {
            unreachable!()
        };
        if ch.is_ascii_whitespace() {
            if !last.ends_with(' ') {
                last.push(' ');
            }
        } else {
            last.push(ch);
        }
    }
    if matches!(pieces.last(), Some(Piece::Text(last)) if last.is_empty()) {
        pieces.pop();
    }
}

/// Flatten the candidate to pieces and apply the filters.
fn build(tree: &Tree<Node>, infos: &HashMap<NodeId, Info>, nodes: Vec<NodeId>) -> Option<Planned> {
    let mut pieces: Vec<Piece> = Vec::new();
    let mut slots: Vec<Slot> = Vec::new();
    let mut open: Vec<u16> = Vec::new();
    // Inside an opaque object: wait for its closing edge.
    let mut inside: Option<NodeId> = None;
    for &top in &nodes {
        for edge in tree.get(top)?.traverse() {
            if let Some(object) = inside {
                if matches!(edge, Edge::Close(node) if node.id() == object) {
                    inside = None;
                }
                continue;
            }
            match edge {
                Edge::Open(node) => match node.value() {
                    Node::Text(text) => push_text(&mut pieces, &text.text),
                    Node::Element(element) => {
                        let kind = kind_of(infos, node)?;
                        if !matches!(kind, Kind::Inline | Kind::Br | Kind::Object) {
                            return None;
                        }
                        if slots.len() >= MAX_PLACEHOLDERS {
                            return None;
                        }
                        let n = slots.len() as u16 + 1;
                        let (slot_kind, piece, required) = match kind {
                            Kind::Inline => (SlotKind::Open, Piece::Open(n), element.name() == "a"),
                            Kind::Br => (SlotKind::Break, Piece::Break(n), false),
                            _ => (SlotKind::Object, Piece::Object(n), carries_content(node)),
                        };
                        slots.push(Slot {
                            kind: slot_kind,
                            node: node.id(),
                            required,
                        });
                        if kind == Kind::Inline {
                            open.push(n);
                        } else if kind == Kind::Object {
                            inside = Some(node.id());
                        }
                        pieces.push(piece);
                    }
                    _ => {}
                },
                Edge::Close(node) => {
                    if kind_of(infos, node) == Some(Kind::Inline) {
                        pieces.push(Piece::Close(open.pop()?));
                    }
                }
            }
        }
    }
    let mut lead = false;
    let mut trail = false;
    if let Some(Piece::Text(text)) = pieces.first_mut() {
        let trimmed = text.trim_start_matches(' ').len();
        lead = trimmed != text.len();
        text.drain(..text.len() - trimmed);
        if text.is_empty() {
            pieces.remove(0);
        }
    }
    if let Some(Piece::Text(text)) = pieces.last_mut() {
        let trimmed = text.trim_end_matches(' ').len();
        trail = trimmed != text.len();
        text.truncate(trimmed);
        if text.is_empty() {
            pieces.pop();
        }
    }
    let text: String = pieces
        .iter()
        .filter_map(|piece| match piece {
            Piece::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    if !text.chars().any(char::is_alphabetic) || is_bare_link(&text) {
        return None;
    }
    Some(Planned {
        nodes,
        slots,
        segment: Segment::new(pieces),
        lead,
        trail,
    })
}

/// Losing an image or code span would lose content; an empty decorative
/// element (or `wbr`) may be dropped by the translation.
fn carries_content(node: Ref) -> bool {
    node.descendants().any(|descendant| {
        descendant
            .value()
            .as_element()
            .is_some_and(|element| matches!(element.name(), "img" | "code" | "kbd" | "samp"))
    })
}

/// The text is just one URL or one e-mail address.
fn is_bare_link(text: &str) -> bool {
    let text = text.trim();
    if text.is_empty() || text.chars().any(char::is_whitespace) {
        return false;
    }
    let lower = text.to_ascii_lowercase();
    if ["http://", "https://", "www."]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
    {
        return true;
    }
    text.split_once('@').is_some_and(|(local, domain)| {
        !local.is_empty()
            && !domain.contains('@')
            && domain.contains('.')
            && !domain.starts_with('.')
            && !domain.ends_with('.')
    })
}
