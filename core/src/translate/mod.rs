//! Pure building blocks for translating mail bodies: segmentation of sanitized
//! HTML, a wire format for translation services, and offline language detection.
//!
//! Nothing here touches the network, storage or settings. The HTML comes from the
//! sanitizer and the translations from a third party, so they are untrusted:
//! translated text only ever becomes text nodes, and the only elements in the
//! output are ones that already existed in the input.
use scraper::Html;

mod detect;
pub mod languages;
mod rewrite;
mod walk;
pub mod wire;

pub use detect::{Detection, detect_language};

/// Bump whenever the walk could yield different segments for the same HTML.
pub const SEGMENTER_VERSION: u32 = 1;
pub const MAX_HTML_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_SEGMENTS: usize = 5000;

/// One element of a segment. The numbers are placeholders, counted from 1 in
/// document order within a segment, shared by all kinds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Piece {
    Text(String),
    Open(u16),
    Close(u16),
    Break(u16),
    Object(u16),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pieces: Vec<Piece>,
}

impl Segment {
    pub(crate) fn new(pieces: Vec<Piece>) -> Self {
        Self { pieces }
    }

    pub fn pieces(&self) -> &[Piece] {
        &self.pieces
    }

    /// Text only: placeholders are dropped and a break becomes a newline.
    pub fn plain_text(&self) -> String {
        pieces_text(&self.pieces)
    }

    pub fn char_count(&self) -> usize {
        self.plain_text().chars().count()
    }
}

pub(crate) fn pieces_text(pieces: &[Piece]) -> String {
    let mut out = String::new();
    for piece in pieces {
        match piece {
            Piece::Text(text) => out.push_str(text),
            Piece::Break(_) => out.push('\n'),
            _ => {}
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExtractError {
    TooLarge,
}

/// Parse, normalize pre-formatted text and plan, exactly the same for both
/// entry points so they can never disagree about segment boundaries.
fn analyze(html: &str, include_quotes: bool) -> (Html, Vec<walk::Planned>) {
    let mut document = Html::parse_fragment(html);
    walk::prepare(&mut document, include_quotes);
    let plans = walk::plan(&document, include_quotes);
    (document, plans)
}

/// Segments in document order. `include_quotes == false` skips the folded
/// quote (`details.mailx-quote`) subtrees.
pub fn extract(html: &str, include_quotes: bool) -> Result<Vec<Segment>, ExtractError> {
    if html.len() > MAX_HTML_BYTES {
        return Err(ExtractError::TooLarge);
    }
    let (_, plans) = analyze(html, include_quotes);
    Ok(plans.into_iter().map(|plan| plan.segment).collect())
}

/// `translations[i]` is the wire-format translation of segment `i`, or `None`
/// to leave it as is. Returns `None` when the count differs from what
/// `extract` yields for this HTML (stale cache) or the HTML is too large.
pub fn apply(html: &str, include_quotes: bool, translations: &[Option<String>]) -> Option<String> {
    if html.len() > MAX_HTML_BYTES {
        return None;
    }
    let (mut document, plans) = analyze(html, include_quotes);
    if plans.len() != translations.len() {
        return None;
    }
    for (plan, translation) in plans.iter().zip(translations) {
        if let Some(wire) = translation {
            rewrite::rewrite(&mut document.tree, plan, wire);
        }
    }
    Some(document.root_element().inner_html())
}

/// Up to `max_chars` of plain text from the segments, in order, one per line.
pub fn sample_text(segments: &[Segment], max_chars: usize) -> String {
    let mut out = String::new();
    let mut count = 0;
    for segment in segments {
        if count >= max_chars {
            break;
        }
        if !out.is_empty() {
            out.push('\n');
            count += 1;
        }
        for ch in segment.plain_text().chars() {
            if count >= max_chars {
                break;
            }
            out.push(ch);
            count += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests;
