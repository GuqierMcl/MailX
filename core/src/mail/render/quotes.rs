//! Display-time folding of the quoted earlier message at the end of a reply.
//!
//! This runs on an already sanitized fragment, only when a reader document is
//! served. It never touches stored bodies, search text or reply/forward drafts.
//! The inserted `<details>`/`<summary>` are host-generated and the label comes
//! from MailX's own localization, never from the message. Recognition is best
//! effort: anything unrecognized, or any doubt, returns the input unchanged.
//!
//! Safety rules for the single folded region: it must be trailing (nothing
//! meaningful after it), something must remain visible before it, it must not
//! be a forwarded message, and no other quote block may sit outside it (that is
//! an interleaved reply).
use std::collections::HashSet;

use ego_tree::{NodeId, NodeRef};
use html5ever::QualName;
use scraper::{
    Html, Node,
    node::{Element, Text},
};

const MAX_INPUT_BYTES: usize = 2 * 1024 * 1024;
/// Candidate validations per message; each one is linear in the document.
const MAX_TRIES: usize = 48;
const HEAD_CHARS: usize = 300;
const LINE_LIMIT: usize = 2000;
const MAX_ATTRIBUTION_CHARS: usize = 300;

type Ref<'a> = NodeRef<'a, Node>;

/// Fold the trailing quoted message of `html` behind a native `<details>`.
/// Returns the input unchanged when nothing is recognized or it is too large.
pub fn fold_quotes(html: &str, label: &str) -> String {
    if label.is_empty() || html.len() > MAX_INPUT_BYTES || html.trim().is_empty() {
        return html.to_owned();
    }
    let mut document = Html::parse_fragment(html);
    let Some(region) = find(&document) else {
        return html.to_owned();
    };
    apply(&mut document, region, label);
    document.root_element().inner_html()
}

/// Sibling run `first..=last` under one parent. When `split` is non-zero the
/// run starts inside the text node `first`, at that byte offset.
#[derive(Clone, Copy)]
struct Region {
    first: NodeId,
    split: usize,
    last: NodeId,
}

fn find(document: &Html) -> Option<Region> {
    let mut budget = MAX_TRIES;
    let layers: [fn(&Html) -> Vec<Region>; 3] = [known_containers, quote_blocks, separators];
    for layer in layers {
        for region in layer(document) {
            if budget == 0 {
                return None;
            }
            budget -= 1;
            if valid(document, region) {
                return Some(region);
            }
        }
    }
    None
}

// ---------------------------------------------------------------- candidates

fn to_end(node: Ref) -> Option<Region> {
    Some(Region {
        first: node.id(),
        split: 0,
        last: node.parent()?.last_child()?.id(),
    })
}

fn single(node: Ref) -> Region {
    Region {
        first: node.id(),
        split: 0,
        last: node.id(),
    }
}

/// Layer 1: containers whose class or id survives sanitization. Document order
/// puts the outermost first, so nested duplicates (`blockquote.gmail_quote`
/// inside `div.gmail_quote`) are only tried if the outer one is rejected.
fn known_containers(document: &Html) -> Vec<Region> {
    const CLASSES: [&str; 6] = [
        "gmail_quote",
        "gmail_extra",
        "yahoo_quoted",
        // Unverified against real messages: documented by the vendors' markup.
        "protonmail_quote",
        "tutanota_quote",
        "zmail_extra",
    ];
    let mut out = Vec::new();
    for node in document.root_element().descendants() {
        let Some(element) = node.value().as_element() else {
            continue;
        };
        let by_class = CLASSES
            .iter()
            .any(|class| element.classes().any(|have| have == *class));
        if element.name() == "blockquote"
            && element
                .attr("type")
                .is_some_and(|kind| kind.eq_ignore_ascii_case("cite"))
        {
            // Apple Mail and Thunderbird, with an optional `moz-cite-prefix`.
            out.push(with_attribution(node));
        } else if by_class {
            out.push(single(node));
        } else {
            match element.id().unwrap_or_default() {
                // Outlook on the web: separator and every following sibling.
                "divRplyFwdMsg" | "oriMsgHtmlSeperator" => {
                    out.extend(to_end(node).map(|region| extend_back_over_rules(document, region)))
                }
                // Unverified (NetEase / 139 / generic): element is the quote.
                "isReplyContent" | "reply139content" | "origbody" => out.push(single(node)),
                // `#mailcontent` is deliberately not recognized: without
                // context it is too generic to treat as a quote.
                _ => {}
            }
        }
    }
    out
}

/// Layers 2 and 4: any blockquote, with its attribution line when there is
/// one directly before it. Document order tries the outermost blockquote first.
fn quote_blocks(document: &Html) -> Vec<Region> {
    document
        .root_element()
        .descendants()
        .filter(|node| matches!(node.value().as_element(), Some(e) if e.name() == "blockquote"))
        .map(with_attribution)
        .collect()
}

fn with_attribution(blockquote: Ref) -> Region {
    let mut region = single(blockquote);
    if let Some((first, split)) = attribution_before(blockquote) {
        region.first = first;
        region.split = split;
    }
    region
}

/// The attribution directly before `node`, ignoring blank nodes and `<br>`.
fn attribution_before(node: Ref) -> Option<(NodeId, usize)> {
    let mut previous = node.prev_sibling();
    while let Some(candidate) = previous {
        if is_blank(candidate) {
            previous = candidate.prev_sibling();
            continue;
        }
        return match candidate.value() {
            Node::Element(element) => (element.classes().any(|c| c == "moz-cite-prefix")
                || is_attribution(&flat_text(candidate, MAX_ATTRIBUTION_CHARS + 100)))
            .then_some((candidate.id(), 0)),
            Node::Text(text) => text_attribution(&text.text).map(|at| (candidate.id(), at)),
            _ => None,
        };
    }
    None
}

/// Plain-text mails put the attribution in the same text node as the reply:
/// find it among the last three lines (wrapped attributions).
fn text_attribution(text: &str) -> Option<usize> {
    let trimmed = text.trim_end_matches(is_space);
    let mut starts = vec![0];
    starts.extend(trimmed.match_indices('\n').map(|(at, _)| at + 1));
    starts
        .into_iter()
        .rev()
        .take(3)
        .find(|&start| is_attribution(&trimmed[start..]))
}

/// Layer 3: separator lines and header blocks, found on the flattened lines of
/// every flow container, then extended or climbed to a wrapper.
fn separators(document: &Html) -> Vec<Region> {
    let mut out = Vec::new();
    for parent in document.root_element().descendants() {
        if !parent.value().is_element() {
            continue;
        }
        let lines = lines_of(parent, is_pre(parent));
        let labels: Vec<Option<Label>> = lines.iter().map(|line| label(&line.text)).collect();
        for (index, line) in lines.iter().enumerate() {
            let hit = if is_separator(&line.text) {
                true
            } else if labels[index].is_some() && (index == 0 || labels[index - 1].is_none()) {
                header_run(&labels[index..])
            } else {
                false
            };
            if !hit {
                continue;
            }
            let Some(last) = parent.last_child() else {
                continue;
            };
            let region = Region {
                first: line.first,
                split: line.offset,
                last: last.id(),
            };
            push_with_wrappers(document, region, &mut out);
        }
    }
    out
}

/// The region itself, then up to three wrapper levels when the region starts at
/// the first meaningful child of its parent (Outlook's `border-top` header
/// wrapper, Foxmail header divs): the quoted body follows the wrapper.
fn push_with_wrappers(document: &Html, region: Region, out: &mut Vec<Region>) {
    let allowed = |region: &Region| {
        document
            .tree
            .get(region.first)
            .and_then(|node| node.parent())
            .and_then(|parent| parent.value().as_element())
            .is_some_and(|element| allowed_parent(element.name()))
    };
    let mut region = extend_back_over_rules(document, region);
    if allowed(&region) {
        out.push(region);
    }
    for _ in 0..3 {
        let Some(first) = document.tree.get(region.first) else {
            return;
        };
        if region.split != 0 && !is_text_prefix_blank(first, region.split) {
            return;
        }
        if !first.prev_siblings().all(is_blank) {
            return;
        }
        let Some(parent) = first.parent() else { return };
        if parent.id() == document.root_element().id() {
            return;
        }
        let Some(next) = to_end(parent) else { return };
        region = extend_back_over_rules(document, next);
        if allowed(&region) {
            out.push(region);
        }
    }
}

fn is_text_prefix_blank(node: Ref, split: usize) -> bool {
    node.value().as_text().is_some_and(|text| {
        text.text
            .get(..split)
            .is_some_and(|p| p.chars().all(is_space))
    })
}

/// Include a directly preceding `<hr>` or Outlook's `#appendonsend` marker.
fn extend_back_over_rules(document: &Html, mut region: Region) -> Region {
    if region.split != 0 {
        return region;
    }
    let Some(mut node) = document.tree.get(region.first) else {
        return region;
    };
    while let Some(previous) = node.prev_sibling() {
        let marker = previous
            .value()
            .as_element()
            .is_some_and(|element| element.name() == "hr" || element.id() == Some("appendonsend"));
        if marker {
            region.first = previous.id();
            node = previous;
        } else if is_blank(previous) {
            node = previous;
        } else {
            break;
        }
    }
    // Blank nodes skipped above only matter if a marker was found after them.
    region
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Label {
    From,
    Date,
    To,
    Cc,
    Subject,
    SubjectForward,
}

fn label(line: &str) -> Option<Label> {
    let line = line.trim_start();
    let end = line.find([':', '：'])?;
    if end > 40 {
        return None;
    }
    let name = line[..end].trim().to_lowercase();
    let value = line[end..]
        .trim_start_matches([':', '：'])
        .trim_start()
        .to_lowercase();
    Some(match name.as_str() {
        "from" | "发件人" | "寄件者" | "寄件人" => Label::From,
        "sent" | "date" | "发送时间" | "发送日期" | "时间" | "日期" | "寄件日期" => {
            Label::Date
        }
        "to" | "收件人" | "收件者" => Label::To,
        "cc" | "抄送" | "抄送人" | "副本" => Label::Cc,
        "subject" | "主题" | "主旨" => {
            if [
                "fw:",
                "fwd:",
                "fw：",
                "fwd：",
                "转发:",
                "转发：",
                "转:",
                "转：",
            ]
            .iter()
            .any(|prefix| value.starts_with(prefix))
            {
                Label::SubjectForward
            } else {
                Label::Subject
            }
        }
        _ => return None,
    })
}

/// At least two distinct labels, including a sender or a subject, and not the
/// header of a forwarded message.
fn header_run(labels: &[Option<Label>]) -> bool {
    let run: Vec<Label> = labels.iter().map_while(|label| *label).collect();
    let distinct: HashSet<Label> = run.iter().copied().collect();
    distinct.len() >= 2
        && !distinct.contains(&Label::SubjectForward)
        && (distinct.contains(&Label::From) || distinct.contains(&Label::Subject))
}

struct Line {
    text: String,
    first: NodeId,
    offset: usize,
}

fn is_block(name: &str) -> bool {
    matches!(
        name,
        "div"
            | "p"
            | "table"
            | "tbody"
            | "thead"
            | "tfoot"
            | "tr"
            | "td"
            | "th"
            | "ul"
            | "ol"
            | "li"
            | "dl"
            | "dt"
            | "dd"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "blockquote"
            | "pre"
            | "section"
            | "article"
            | "header"
            | "footer"
            | "main"
            | "center"
            | "address"
            | "hr"
            | "details"
    )
}

fn is_pre(node: Ref) -> bool {
    std::iter::once(node).chain(node.ancestors()).any(|n| {
        n.value()
            .as_element()
            .and_then(|e| e.attr("style"))
            .is_some_and(|style| {
                let style = style.to_ascii_lowercase();
                style.contains("white-space") && style.contains("pre")
            })
    })
}

/// Lines of one parent: `<br>`, block children and (when `pre`) newlines break
/// lines. A block child is one line of its flattened text, so nested blocks are
/// examined when their own turn comes as a parent.
fn lines_of(parent: Ref, pre: bool) -> Vec<Line> {
    let mut out = Vec::new();
    let mut current: Option<Line> = None;
    let flush = |current: &mut Option<Line>, out: &mut Vec<Line>| {
        if let Some(line) = current.take() {
            out.push(line);
        }
    };
    for child in parent.children() {
        match child.value() {
            Node::Text(text) => {
                let text: &str = &text.text;
                let segments: Vec<(usize, &str)> = if pre {
                    let mut at = 0;
                    text.split('\n')
                        .map(|segment| {
                            let start = at;
                            at += segment.len() + 1;
                            (start, segment)
                        })
                        .collect()
                } else {
                    vec![(0, text)]
                };
                for (index, (offset, segment)) in segments.into_iter().enumerate() {
                    if index > 0 {
                        flush(&mut current, &mut out);
                    }
                    if segment.chars().all(is_space) {
                        continue;
                    }
                    match &mut current {
                        Some(line) => {
                            if line.text.len() < LINE_LIMIT {
                                line.text.push(' ');
                                line.text.push_str(segment.trim());
                            }
                        }
                        None => {
                            current = Some(Line {
                                text: segment.trim().to_owned(),
                                first: child.id(),
                                offset,
                            })
                        }
                    }
                }
            }
            Node::Element(element) => {
                let name = element.name();
                if name == "style" {
                    continue;
                }
                if name == "br" {
                    flush(&mut current, &mut out);
                    continue;
                }
                let block = is_block(name);
                if block {
                    flush(&mut current, &mut out);
                }
                let text = flat_text(child, LINE_LIMIT);
                if text.is_empty() {
                    continue;
                }
                match (&mut current, block) {
                    (Some(line), false) => {
                        if line.text.len() < LINE_LIMIT {
                            line.text.push(' ');
                            line.text.push_str(&text);
                        }
                    }
                    (_, false) => {
                        current = Some(Line {
                            text,
                            first: child.id(),
                            offset: 0,
                        })
                    }
                    (_, true) => out.push(Line {
                        text,
                        first: child.id(),
                        offset: 0,
                    }),
                }
            }
            _ => {}
        }
    }
    flush(&mut current, &mut out);
    out
}

/// `-----Original Message-----`, `------ 原始邮件 ------`, `--- Original ---` and
/// a few other languages. Needs at least three decoration characters so a
/// sentence that merely says "original" is never a separator.
fn is_separator(line: &str) -> bool {
    let line = normalize(line).to_lowercase();
    let decoration = |c: char| matches!(c, '-' | '—' | '–' | '_' | '=' | '*' | '~' | '·' | '•');
    let marks = line.chars().filter(|c| decoration(*c)).count();
    let core = line.trim_matches(|c: char| decoration(c) || c == ' ');
    marks >= 3
        && matches!(
            core,
            "original message"
                | "original"
                | "original mail"
                | "original email"
                | "original e-mail"
                | "原始邮件"
                | "原邮件"
                | "原始信息"
                | "原始消息"
                | "ursprüngliche nachricht"
                | "message d'origine"
                | "mensaje original"
                | "元のメッセージ"
        )
}

/// "On … wrote:", "… 写道：", "Am … schrieb …:", "Le … a écrit :", "El …
/// escribió:". Short, ends with a colon, and has no sentence punctuation, so
/// text the user wrote on the same line is never taken for an attribution.
fn is_attribution(text: &str) -> bool {
    let text = normalize(text);
    if text.is_empty() || text.chars().count() > MAX_ATTRIBUTION_CHARS {
        return false;
    }
    if !text.ends_with([':', '：']) || text.contains(['。', '！', '？', '!', '?']) {
        return false;
    }
    let lower = text.to_lowercase();
    let core = lower.trim_end_matches([':', '：', ' ']);
    let starts = |prefix: &str| core.starts_with(prefix);
    let contact = core.contains(['@', '<', '＜']);
    if core.ends_with("wrote") {
        return starts("on ") || (contact && core.chars().count() <= 160 && !core.contains(". "));
    }
    if core.ends_with("写道") || core.ends_with("写到") {
        return starts("在")
            || starts("on ")
            || core.chars().next().is_some_and(|c| c.is_ascii_digit())
            || contact;
    }
    if starts("am ") && core.contains(" schrieb ") {
        return true;
    }
    if starts("le ") && core.ends_with("a écrit") {
        return true;
    }
    if starts("el ") && core.ends_with("escribió") {
        return true;
    }
    core.ends_with("書きました") && core.chars().count() <= 160
}

// ---------------------------------------------------------------- validation

fn allowed_parent(name: &str) -> bool {
    // Flow containers only: never inside tables, lists, paragraphs or headings,
    // where a block `<details>` would change the structure.
    matches!(
        name,
        "html"
            | "div"
            | "td"
            | "th"
            | "section"
            | "main"
            | "article"
            | "blockquote"
            | "li"
            | "center"
            | "font"
    )
}

fn valid(document: &Html, region: Region) -> bool {
    let tree = &document.tree;
    let Some(first) = tree.get(region.first) else {
        return false;
    };
    let Some(parent) = first.parent() else {
        return false;
    };
    if !parent
        .value()
        .as_element()
        .is_some_and(|element| allowed_parent(element.name()))
    {
        return false;
    }
    // Sibling run first..=last.
    let mut tops = vec![first];
    while tops.last().map(|n| n.id()) != Some(region.last) {
        match tops.last().and_then(|n| n.next_sibling()) {
            Some(next) => tops.push(next),
            None => return false,
        }
    }
    let inside: HashSet<NodeId> = tops
        .iter()
        .flat_map(|top| top.descendants().map(|n| n.id()))
        .collect();
    // Never fold a forwarded message, nor an empty region.
    if inside.iter().any(|id| {
        tree.get(*id)
            .and_then(|n| n.value().as_element())
            .is_some_and(|e| e.id() == Some("isForwardContent"))
    }) {
        return false;
    }
    let head = region_head(&tops, region.split);
    if head.trim().is_empty() && !tops.iter().any(|top| has_image(*top)) {
        return false;
    }
    // Trailing only: everything after the region, up to the root, is blank.
    let mut node = tree.get(region.last).unwrap();
    loop {
        if !node.next_siblings().all(is_blank) {
            return false;
        }
        match node.parent() {
            Some(up) => node = up,
            None => break,
        }
    }
    // What stays visible, and any other quote outside the region.
    let mut has_content = false;
    let mut before = String::new();
    for node in document.root_element().descendants() {
        let id = node.id();
        match node.value() {
            Node::Text(text) => {
                if node.parent().is_some_and(|p| is_named(p, "style")) {
                    continue;
                }
                let visible: &str = if id == region.first && region.split > 0 {
                    text.text.get(..region.split).unwrap_or("")
                } else if inside.contains(&id) {
                    continue;
                } else {
                    &text.text
                };
                if visible.chars().any(|c| !is_space(c)) {
                    has_content = true;
                }
                before.push_str(visible);
                before.push(' ');
            }
            Node::Element(element) => {
                if inside.contains(&id) {
                    continue;
                }
                if element.name() == "img" {
                    has_content = true;
                } else if element.name() == "blockquote" && !is_blank(node) {
                    return false;
                }
            }
            _ => {}
        }
    }
    if !has_content {
        return false;
    }
    !is_forward(&before, &head)
}

fn is_named(node: Ref, name: &str) -> bool {
    node.value().as_element().is_some_and(|e| e.name() == name)
}

fn has_image(node: Ref) -> bool {
    node.descendants().any(|n| is_named(n, "img"))
}

/// Visible text at the start of the region (lowercase, collapsed).
fn region_head(tops: &[Ref], split: usize) -> String {
    let mut out = String::new();
    for (index, top) in tops.iter().enumerate() {
        if out.chars().count() >= HEAD_CHARS {
            break;
        }
        out.push(' ');
        match top.value() {
            Node::Text(text) => out.push_str(
                text.text
                    .get(if index == 0 { split } else { 0 }..)
                    .unwrap_or(""),
            ),
            _ => out.push_str(&flat_text(*top, HEAD_CHARS)),
        }
    }
    normalize(&out).to_lowercase()
}

fn is_forward(before: &str, head: &str) -> bool {
    const MARKERS: [&str; 6] = [
        "forwarded message",
        "begin forwarded",
        "转发邮件信息",
        "转发的邮件",
        "转发邮件",
        "已转发",
    ];
    let before = normalize(before).to_lowercase();
    let tail: String = {
        let mut chars: Vec<char> = before.chars().rev().take(200).collect();
        chars.reverse();
        chars.into_iter().collect()
    };
    let head: String = head.chars().take(HEAD_CHARS).collect();
    MARKERS
        .iter()
        .any(|marker| tail.contains(marker) || head.contains(marker))
}

// ----------------------------------------------------------------- mutation

fn qual(name: &str) -> QualName {
    QualName::new(
        None,
        "http://www.w3.org/1999/xhtml".into(),
        name.to_owned().into(),
    )
}

fn apply(document: &mut Html, region: Region, label: &str) {
    let tree = &mut document.tree;
    let mut first = region.first;
    let mut last = region.last;
    if region.split > 0 {
        let tail = match tree.get_mut(first).unwrap().value() {
            Node::Text(text) => {
                let full = text.text.to_string();
                let (head, tail) = full.split_at(region.split);
                text.text = head.to_owned().into();
                tail.to_owned()
            }
            _ => return,
        };
        let new = tree.orphan(Node::Text(Text { text: tail.into() })).id();
        tree.get_mut(first).unwrap().insert_id_after(new);
        if last == first {
            last = new;
        }
        first = new;
    }
    // Collect the run before moving anything.
    let mut run = vec![first];
    while *run.last().unwrap() != last {
        match tree
            .get(*run.last().unwrap())
            .and_then(|n| n.next_sibling())
        {
            Some(next) => run.push(next.id()),
            None => return,
        }
    }
    let mut details = Element::new(qual("details"), vec![]);
    details.attrs.push((
        QualName::new(None, "".into(), "class".into()),
        "mailx-quote".into(),
    ));
    let details = tree.orphan(Node::Element(details)).id();
    let summary = tree
        .orphan(Node::Element(Element::new(qual("summary"), vec![])))
        .id();
    tree.get_mut(summary).unwrap().append(Node::Text(Text {
        text: label.to_owned().into(),
    }));
    tree.get_mut(first).unwrap().insert_id_before(details);
    tree.get_mut(details).unwrap().append_id(summary);
    for id in run {
        tree.get_mut(details).unwrap().append_id(id);
    }
}

// ------------------------------------------------------------------- helpers

fn is_space(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '\u{200b}' | '\u{200c}' | '\u{200d}' | '\u{feff}' | '\u{2060}' | '\u{00ad}'
        )
}

fn normalize(text: &str) -> String {
    text.split(is_space)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Text of a subtree, `<br>` and block boundaries as spaces, `style` skipped,
/// collapsed; stops once `limit` characters were gathered.
fn flat_text(node: Ref, limit: usize) -> String {
    let mut out = String::new();
    for descendant in node.descendants() {
        match descendant.value() {
            Node::Text(text) => {
                if descendant.parent().is_some_and(|p| is_named(p, "style")) {
                    continue;
                }
                out.push_str(&text.text);
            }
            Node::Element(element) if element.name() == "br" || is_block(element.name()) => {
                out.push(' ')
            }
            _ => {}
        }
        if out.len() >= limit {
            break;
        }
    }
    normalize(&out)
}

/// No visible text and no image anywhere in the subtree. Iterative, so hostile
/// nesting cannot overflow the stack.
fn is_blank(node: Ref) -> bool {
    node.descendants()
        .all(|descendant| match descendant.value() {
            Node::Text(text) => {
                descendant.parent().is_some_and(|p| is_named(p, "style"))
                    || text.text.chars().all(is_space)
            }
            Node::Element(element) => !matches!(
                element.name(),
                "img"
                    | "video"
                    | "audio"
                    | "canvas"
                    | "svg"
                    | "object"
                    | "embed"
                    | "iframe"
                    | "input"
            ),
            _ => true,
        })
}

#[cfg(test)]
mod tests;
