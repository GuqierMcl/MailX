//! Put a translation back into the document.
//!
//! The translation is untrusted. It never creates elements: text becomes text
//! nodes, and the only elements in the result are shallow copies of elements
//! that were in the source segment (once each) or the source nodes themselves.
use std::collections::HashSet;

use ego_tree::{NodeId, Tree};
use scraper::{
    Node,
    node::{Element, Text},
};

use super::{
    Piece, pieces_text,
    walk::{Planned, SlotKind, qual},
    wire,
};

pub(super) fn rewrite(tree: &mut Tree<Node>, plan: &Planned, translation: &str) {
    let decoded = wire::decode(translation);
    let has_text = decoded
        .iter()
        .any(|piece| matches!(piece, Piece::Text(text) if !text.trim().is_empty()));
    if !has_text {
        return;
    }
    let mut pieces = if fits(plan, &decoded) {
        decoded
    } else if plan.slots.iter().any(|slot| slot.required) {
        // Losing a link or an image is worse than leaving the original.
        return;
    } else {
        vec![Piece::Text(pieces_text(&decoded))]
    };
    restore_edges(&mut pieces, plan.lead, plan.trail);
    splice(tree, plan, &pieces);
}

/// Every placeholder exists in the source with the same kind and is used at
/// most once, and every required one is present.
fn fits(plan: &Planned, pieces: &[Piece]) -> bool {
    // 0 unused, 1 open, 2 done.
    let mut state = vec![0u8; plan.slots.len()];
    let index = |n: u16, kind: SlotKind| {
        let at = usize::from(n).checked_sub(1)?;
        plan.slots
            .get(at)
            .filter(|slot| slot.kind == kind)
            .map(|_| at)
    };
    for piece in pieces {
        match *piece {
            Piece::Text(_) => {}
            Piece::Open(n) => {
                let Some(at) = index(n, SlotKind::Open) else {
                    return false;
                };
                if state[at] != 0 {
                    return false;
                }
                state[at] = 1;
            }
            Piece::Close(n) => {
                let Some(at) = index(n, SlotKind::Open) else {
                    return false;
                };
                if state[at] != 1 {
                    return false;
                }
                state[at] = 2;
            }
            Piece::Break(n) | Piece::Object(n) => {
                let kind = if matches!(piece, Piece::Break(_)) {
                    SlotKind::Break
                } else {
                    SlotKind::Object
                };
                let Some(at) = index(n, kind) else {
                    return false;
                };
                if state[at] != 0 {
                    return false;
                }
                state[at] = 2;
            }
        }
    }
    plan.slots
        .iter()
        .zip(&state)
        .all(|(slot, state)| !slot.required || *state == 2)
}

/// Trim the translation's outer whitespace, then give back the single space
/// the source had at each end.
fn restore_edges(pieces: &mut Vec<Piece>, lead: bool, trail: bool) {
    if let Some(Piece::Text(text)) = pieces.first_mut() {
        *text = text.trim_start().to_owned();
        if text.is_empty() {
            pieces.remove(0);
        }
    }
    if let Some(Piece::Text(text)) = pieces.last_mut() {
        *text = text.trim_end().to_owned();
        if text.is_empty() {
            pieces.pop();
        }
    }
    if lead {
        match pieces.first_mut() {
            Some(Piece::Text(text)) => text.insert(0, ' '),
            _ => pieces.insert(0, Piece::Text(" ".to_owned())),
        }
    }
    if trail {
        match pieces.last_mut() {
            Some(Piece::Text(text)) => text.push(' '),
            _ => pieces.push(Piece::Text(" ".to_owned())),
        }
    }
}

fn splice(tree: &mut Tree<Node>, plan: &Planned, pieces: &[Piece]) {
    // Marks the spot; the old nodes may include the first one that moves.
    let marker = tree.orphan(Node::Text(Text { text: "".into() })).id();
    tree.get_mut(plan.nodes[0])
        .unwrap()
        .insert_id_before(marker);
    let holder = tree
        .orphan(Node::Element(Element::new(qual("span"), vec![])))
        .id();
    let mut parents = vec![holder];
    let mut moved: HashSet<NodeId> = HashSet::new();
    for piece in pieces {
        let parent = *parents.last().unwrap();
        match piece {
            Piece::Text(text) => {
                let node = tree
                    .orphan(Node::Text(Text {
                        text: text.clone().into(),
                    }))
                    .id();
                tree.get_mut(parent).unwrap().append_id(node);
            }
            Piece::Open(n) => {
                let source = plan.slots[usize::from(*n) - 1].node;
                let copy = tree
                    .get(source)
                    .and_then(|node| node.value().as_element())
                    .cloned()
                    .expect("slot is an element");
                let node = tree.orphan(Node::Element(copy)).id();
                tree.get_mut(parent).unwrap().append_id(node);
                parents.push(node);
            }
            Piece::Close(_) => {
                if parents.len() > 1 {
                    parents.pop();
                }
            }
            Piece::Break(n) | Piece::Object(n) => {
                let source = plan.slots[usize::from(*n) - 1].node;
                moved.insert(source);
                tree.get_mut(parent).unwrap().append_id(source);
            }
        }
    }
    let built: Vec<NodeId> = tree
        .get(holder)
        .unwrap()
        .children()
        .map(|node| node.id())
        .collect();
    for node in built {
        tree.get_mut(marker).unwrap().insert_id_before(node);
    }
    for node in &plan.nodes {
        if !moved.contains(node) {
            tree.get_mut(*node).unwrap().detach();
        }
    }
    tree.get_mut(marker).unwrap().detach();
}
