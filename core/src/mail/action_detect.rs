//! L1 local rule-based detection of dates, times, and high-signal action
//! phrases (`docs/architecture/action-items.md` §2.2).
//!
//! Pure, bounded, host-independent: never touches the network, storage, or
//! settings. Operates on safe HTML text nodes after quote folding and before
//! resource binding so translation segments and stored sanitized HTML remain
//! untouched.
use chrono::{
    DateTime, Datelike, Duration, FixedOffset, NaiveDate, NaiveTime, TimeZone, Utc, Weekday,
};
use html5ever::{Attribute, LocalName, Namespace, QualName};
use scraper::{
    Html, Node,
    node::{Element, Text},
};
use sha2::{Digest, Sha256};

use super::action_items::{
    ActionItemKind, ActionItemSource, ActionItemStatus, DetectedActionItem, MAX_MATCHED_TEXT_CHARS,
    MAX_TITLE_CHARS, clean_text,
};

pub const ACTION_SLOT: &str = "data-mailx-action";
pub const MAX_SCAN_BYTES: usize = 64 * 1024;
pub const MAX_DETECTED_ITEMS: usize = 15;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSpanMatch {
    pub start: usize,
    pub end: usize,
    pub kind: ActionItemKind,
    pub start_at: Option<String>,
    pub all_day: bool,
    pub matched_text: String,
    pub suggested_title: String,
}

/// Annotates safe text nodes in an already-sanitized HTML fragment with
/// `<a data-mailx-action="<idx>" class="mailx-detected-action" title="...">`
/// and returns the modified HTML along with the ordered list of detected items.
pub fn annotate_html(
    html: &str,
    subject: &str,
    reference_rfc3339: Option<&str>,
    max_items: usize,
) -> (String, Vec<DetectedActionItem>) {
    let limit = max_items.min(MAX_DETECTED_ITEMS);
    if limit == 0 || html.is_empty() || html.len() > MAX_SCAN_BYTES * 4 {
        return (html.to_owned(), Vec::new());
    }

    let reference = reference_rfc3339
        .and_then(|s| DateTime::parse_from_rfc3339(s.trim()).ok())
        .unwrap_or_else(|| Utc::now().fixed_offset());
    let clean_subject = clean_text(subject, MAX_TITLE_CHARS, false);

    let mut document = Html::parse_fragment(html);
    let text_ids: Vec<_> = document
        .tree
        .root()
        .descendants()
        .filter(|node| matches!(node.value(), Node::Text(_)) && !is_skipped_ancestor(*node))
        .map(|node| node.id())
        .collect();

    let mut scanned_bytes = 0usize;
    let mut items: Vec<DetectedActionItem> = Vec::new();

    for node_id in text_ids {
        if items.len() >= limit || scanned_bytes >= MAX_SCAN_BYTES {
            break;
        }
        let text = match document.tree.get(node_id).map(|n| n.value()) {
            Some(Node::Text(t)) => t.text.to_string(),
            _ => continue,
        };
        scanned_bytes = scanned_bytes.saturating_add(text.len());
        let remaining = limit - items.len();
        let matches = scan_text(&text, reference, &clean_subject, remaining);
        if matches.is_empty() {
            continue;
        }

        // Replace text node with alternating Text and <a data-mailx-action> siblings.
        let mut cursor = 0usize;
        let mut fragments: Vec<Node> = Vec::with_capacity(matches.len() * 2 + 1);
        let mut new_anchors: Vec<(usize, String, String)> = Vec::new();
        for m in matches {
            if m.start < cursor || m.end > text.len() || m.start >= m.end {
                continue;
            }
            if m.start > cursor {
                fragments.push(Node::Text(Text {
                    text: text[cursor..m.start].into(),
                }));
            }
            let idx = items.len();
            let item = build_rule_item(idx, &m, &clean_subject);
            let title_attr = item.title.clone();
            items.push(item);

            let attrs = vec![
                Attribute {
                    name: QualName::new(None, Namespace::from(""), LocalName::from(ACTION_SLOT)),
                    value: idx.to_string().into(),
                },
                Attribute {
                    name: QualName::new(None, Namespace::from(""), LocalName::from("class")),
                    value: "mailx-detected-action".into(),
                },
                Attribute {
                    name: QualName::new(None, Namespace::from(""), LocalName::from("title")),
                    value: title_attr.into(),
                },
            ];
            let element = Element::new(
                QualName::new(
                    None,
                    Namespace::from("http://www.w3.org/1999/xhtml"),
                    LocalName::from("a"),
                ),
                attrs,
            );
            let frag_pos = fragments.len();
            fragments.push(Node::Element(element));
            new_anchors.push((frag_pos, text[m.start..m.end].to_owned(), m.matched_text));
            cursor = m.end;
        }
        if cursor < text.len() {
            fragments.push(Node::Text(Text {
                text: text[cursor..].into(),
            }));
        }
        if new_anchors.is_empty() {
            continue;
        }

        let mut anchor_map = std::collections::HashMap::new();
        for (pos, label, _) in new_anchors {
            anchor_map.insert(pos, label);
        }

        if let Some(mut node_mut) = document.tree.get_mut(node_id) {
            for (pos, frag) in fragments.into_iter().enumerate() {
                let mut inserted = node_mut.insert_before(frag);
                if let Some(label) = anchor_map.remove(&pos) {
                    inserted.append(Node::Text(Text { text: label.into() }));
                }
            }
            node_mut.detach();
        }
    }

    (document.root_element().inner_html(), items)
}

fn is_skipped_ancestor(node: ego_tree::NodeRef<'_, Node>) -> bool {
    for ancestor in node.ancestors() {
        if let Node::Element(el) = ancestor.value() {
            let name = el.name();
            if matches!(
                name,
                "a" | "button"
                    | "code"
                    | "pre"
                    | "style"
                    | "script"
                    | "blockquote"
                    | "details"
                    | "summary"
            ) {
                return true;
            }
        }
    }
    false
}

fn build_rule_item(idx: usize, m: &TextSpanMatch, subject: &str) -> DetectedActionItem {
    let mut hasher = Sha256::new();
    hasher.update(m.matched_text.as_bytes());
    if let Some(start) = m.start_at.as_deref() {
        hasher.update(b"|");
        hasher.update(start.as_bytes());
    }
    let digest = format!("{:x}", hasher.finalize());
    let title = if !m.suggested_title.is_empty() {
        m.suggested_title.clone()
    } else if !subject.is_empty() {
        subject.to_owned()
    } else {
        clean_text(&m.matched_text, MAX_TITLE_CHARS, false)
    };
    DetectedActionItem {
        id: format!("rule:{idx}:{}", &digest[..10]),
        kind: m.kind,
        source: ActionItemSource::Rule,
        status: ActionItemStatus::Pending,
        title,
        start_at: m.start_at.clone(),
        end_at: None,
        all_day: m.all_day,
        rrule: None,
        cancelled: false,
        location: None,
        url: None,
        notes: None,
        matched_text: Some(clean_text(&m.matched_text, MAX_MATCHED_TEXT_CHARS, false)),
    }
}

/// Scans a single plain-text segment for date/time expressions and high-signal
/// task phrases while filtering out noise (courtesies, OTP expiry phrases,
/// version numbers, order IDs).
pub fn scan_text(
    text: &str,
    reference: DateTime<FixedOffset>,
    subject: &str,
    limit: usize,
) -> Vec<TextSpanMatch> {
    if limit == 0 || text.trim().is_empty() {
        return Vec::new();
    }
    if is_otp_or_expiry_noise(text) {
        return Vec::new();
    }

    let mut candidates: Vec<TextSpanMatch> = Vec::new();
    find_iso_dates(text, reference, subject, &mut candidates);
    find_cn_dates(text, reference, subject, &mut candidates);
    find_en_dates(text, reference, subject, &mut candidates);
    find_cn_relative_dates(text, reference, subject, &mut candidates);
    find_en_relative_dates(text, reference, subject, &mut candidates);
    find_high_signal_tasks(text, &mut candidates);

    // Sort by start offset, prefer longer span on tie, drop overlapping spans.
    candidates.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| b.end.cmp(&a.end)));
    let mut non_overlapping: Vec<TextSpanMatch> = Vec::new();
    let mut last_end = 0usize;
    for cand in candidates {
        if cand.start >= last_end {
            last_end = cand.end;
            non_overlapping.push(cand);
            if non_overlapping.len() >= limit {
                break;
            }
        }
    }
    non_overlapping
}

fn is_otp_or_expiry_noise(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let otp_word = lower.contains("verification code")
        || lower.contains("security code")
        || lower.contains("one-time")
        || text.contains("验证码")
        || text.contains("校验码")
        || text.contains("动态密码");
    let expiry = lower.contains("expires in")
        || lower.contains("valid for")
        || text.contains("分钟内有效")
        || text.contains("有效期内");
    otp_word && expiry
}

fn is_boundary_before(text: &str, byte_idx: usize) -> bool {
    if byte_idx == 0 {
        return true;
    }
    let Some(ch) = text[..byte_idx].chars().next_back() else {
        return true;
    };
    !ch.is_ascii_alphanumeric() && ch != '_' && ch != '.'
}

fn is_boundary_after(text: &str, byte_idx: usize) -> bool {
    if byte_idx >= text.len() {
        return true;
    }
    let Some(ch) = text[byte_idx..].chars().next() else {
        return true;
    };
    !ch.is_ascii_alphanumeric() && ch != '_' && ch != '.'
}

fn parse_hm_time(s: &str) -> Option<(NaiveTime, usize)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let rest = &s[i..];
    // Optional "at "
    let rest = if rest.len() >= 3 && rest[..3].eq_ignore_ascii_case("at ") {
        i += 3;
        &s[i..]
    } else {
        rest
    };
    // Optional Chinese period prefix
    let (period, prefix_len) = if let Some(r) = rest.strip_prefix("上午") {
        (Some(Period::Am), rest.len() - r.len())
    } else if let Some(r) = rest.strip_prefix("早上") {
        (Some(Period::Am), rest.len() - r.len())
    } else if let Some(r) = rest.strip_prefix("下午") {
        (Some(Period::Pm), rest.len() - r.len())
    } else if let Some(r) = rest.strip_prefix("晚上") {
        (Some(Period::Pm), rest.len() - r.len())
    } else if let Some(r) = rest.strip_prefix("中午") {
        (Some(Period::Pm), rest.len() - r.len())
    } else {
        (None, 0)
    };
    i += prefix_len;
    while i < bytes.len() && bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    let time_part = &s[i..];
    // Try HH:MM (or H:MM)
    let (h, h_len) = take_u32(time_part, 2)?;
    let after_h = &time_part[h_len..];
    if let Some(after_colon) = after_h
        .strip_prefix(':')
        .or_else(|| after_h.strip_prefix('：'))
    {
        let (m, m_len) = take_u32(after_colon, 2)?;
        let mut consumed = i + h_len + (after_h.len() - after_colon.len()) + m_len;
        let (ampm, suffix_len) = take_ampm(&s[consumed..]);
        consumed += suffix_len;
        let hour = adjust_hour(h, ampm.or(period))?;
        let time = NaiveTime::from_hms_opt(hour, m, 0)?;
        return Some((time, consumed));
    }
    // Try Chinese "3点" / "3点半" / "3点30分"
    if let Some(after_dian) = after_h
        .strip_prefix('点')
        .or_else(|| after_h.strip_prefix('时'))
    {
        let mut consumed = i + h_len + (after_h.len() - after_dian.len());
        let min = if let Some(rest_half) = after_dian.strip_prefix('半') {
            consumed += after_dian.len() - rest_half.len();
            30
        } else if let Some((m, m_len)) = take_u32(after_dian, 2) {
            consumed += m_len;
            if after_dian[m_len..].starts_with('分') {
                consumed += '分'.len_utf8();
            }
            m
        } else {
            0
        };
        let hour = adjust_hour(h, period)?;
        let time = NaiveTime::from_hms_opt(hour, min, 0)?;
        return Some((time, consumed));
    }
    // Try English "2pm" / "2 pm"
    let (ampm, suffix_len) = take_ampm(after_h);
    if let Some(p) = ampm {
        let hour = adjust_hour(h, Some(p))?;
        let time = NaiveTime::from_hms_opt(hour, 0, 0)?;
        return Some((time, i + h_len + suffix_len));
    }
    None
}

#[derive(Clone, Copy)]
enum Period {
    Am,
    Pm,
}

fn adjust_hour(h: u32, period: Option<Period>) -> Option<u32> {
    match period {
        Some(Period::Pm) => {
            if h == 0 || h > 23 {
                None
            } else if h < 12 {
                Some(h + 12)
            } else {
                Some(h)
            }
        }
        Some(Period::Am) => {
            if h > 12 {
                None
            } else if h == 12 {
                Some(0)
            } else {
                Some(h)
            }
        }
        None => (h <= 23).then_some(h),
    }
}

fn take_ampm(s: &str) -> (Option<Period>, usize) {
    let trimmed = s.trim_start_matches(' ');
    let ws = s.len() - trimmed.len();
    if trimmed.len() >= 2 {
        let head = &trimmed[..2];
        let after = &trimmed[2..];
        let boundary = after
            .chars()
            .next()
            .is_none_or(|c| !c.is_ascii_alphanumeric());
        if boundary {
            if head.eq_ignore_ascii_case("am") {
                return (Some(Period::Am), ws + 2);
            }
            if head.eq_ignore_ascii_case("pm") {
                return (Some(Period::Pm), ws + 2);
            }
        }
    }
    (None, 0)
}

fn take_u32(s: &str, max_digits: usize) -> Option<(u32, usize)> {
    let mut len = 0;
    let mut val = 0u32;
    for b in s.bytes().take(max_digits) {
        if !b.is_ascii_digit() {
            break;
        }
        val = val * 10 + (b - b'0') as u32;
        len += 1;
    }
    (len > 0).then_some((val, len))
}

fn format_date_or_datetime(
    date: NaiveDate,
    time: Option<NaiveTime>,
    offset: FixedOffset,
) -> (String, bool) {
    match time {
        Some(t) => {
            let naive = date.and_time(t);
            let dt = offset
                .from_local_datetime(&naive)
                .single()
                .unwrap_or_else(|| Utc.from_utc_datetime(&naive).fixed_offset());
            (dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true), false)
        }
        None => (date.format("%Y-%m-%d").to_string(), true),
    }
}

/// Matches `YYYY-MM-DD [HH:MM]` or `YYYY/MM/DD [HH:MM]`.
fn find_iso_dates(
    text: &str,
    reference: DateTime<FixedOffset>,
    subject: &str,
    out: &mut Vec<TextSpanMatch>,
) {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i + 8 <= bytes.len() {
        if bytes[i].is_ascii_digit()
            && is_boundary_before(text, i)
            && let Some((y, 4)) = take_u32(&text[i..], 4)
            && (2020..=2100).contains(&y)
        {
            let sep = bytes.get(i + 4).copied();
            if matches!(sep, Some(b'-') | Some(b'/')) {
                let sep = sep.unwrap();
                if let Some((m, m_len)) = take_u32(&text[i + 5..], 2)
                    && (1..=12).contains(&m)
                    && bytes.get(i + 5 + m_len) == Some(&sep)
                    && let Some((d, d_len)) = take_u32(&text[i + 6 + m_len..], 2)
                    && let Some(date) = NaiveDate::from_ymd_opt(y as i32, m, d)
                {
                    let mut end = i + 6 + m_len + d_len;
                    if is_boundary_after(text, end) {
                        let mut time = None;
                        if let Some((t, t_len)) = parse_hm_time(&text[end..]) {
                            time = Some(t);
                            end += t_len;
                        }
                        let (start_at, all_day) =
                            format_date_or_datetime(date, time, *reference.offset());
                        out.push(TextSpanMatch {
                            start: i,
                            end,
                            kind: ActionItemKind::Event,
                            start_at: Some(start_at),
                            all_day,
                            matched_text: text[i..end].to_owned(),
                            suggested_title: subject.to_owned(),
                        });
                        i = end;
                        continue;
                    }
                }
            }
        }
        i += 1;
        while i < bytes.len() && !text.is_char_boundary(i) {
            i += 1;
        }
    }
}

/// Matches Chinese absolute dates like `2026年12月3日 14:30` or `12月3日下午3点`.
fn find_cn_dates(
    text: &str,
    reference: DateTime<FixedOffset>,
    subject: &str,
    out: &mut Vec<TextSpanMatch>,
) {
    let mut i = 0;
    while i < text.len() {
        let slice = &text[i..];
        let mut cursor = 0usize;
        let mut year = reference.year();
        let mut has_year = false;

        if is_boundary_before(text, i)
            && let Some((y, 4)) = take_u32(slice, 4)
            && (2020..=2100).contains(&y)
            && slice[4..].starts_with('年')
        {
            year = y as i32;
            has_year = true;
            cursor = 4 + '年'.len_utf8();
        }

        if (has_year || is_boundary_before(text, i))
            && let Some((m, m_len)) = take_u32(&slice[cursor..], 2)
            && (1..=12).contains(&m)
            && slice[cursor + m_len..].starts_with('月')
        {
            let after_yue = cursor + m_len + '月'.len_utf8();
            if let Some((d, d_len)) = take_u32(&slice[after_yue..], 2)
                && (1..=31).contains(&d)
            {
                let after_d = after_yue + d_len;
                let day_suffix = if slice[after_d..].starts_with('日') {
                    Some('日'.len_utf8())
                } else if slice[after_d..].starts_with('号') {
                    Some('号'.len_utf8())
                } else {
                    None
                };
                if let Some(suf) = day_suffix
                    && let Some(mut date) = NaiveDate::from_ymd_opt(year, m, d)
                {
                    // If no year was specified and the date is more than 60 days
                    // before the reference date (e.g. "Jan 5" in a December email),
                    // roll forward to next year.
                    if !has_year
                        && date < reference.date_naive() - Duration::days(60)
                        && let Some(next_year_date) = NaiveDate::from_ymd_opt(year + 1, m, d)
                    {
                        date = next_year_date;
                    }
                    let mut end = i + after_d + suf;
                    let mut time = None;
                    if let Some((t, t_len)) = parse_hm_time(&text[end..]) {
                        time = Some(t);
                        end += t_len;
                    }
                    let (start_at, all_day) =
                        format_date_or_datetime(date, time, *reference.offset());
                    out.push(TextSpanMatch {
                        start: i,
                        end,
                        kind: ActionItemKind::Event,
                        start_at: Some(start_at),
                        all_day,
                        matched_text: text[i..end].to_owned(),
                        suggested_title: subject.to_owned(),
                    });
                    i = end;
                    continue;
                }
            }
        }
        i += slice.chars().next().map_or(1, char::len_utf8);
    }
}

/// Matches English dates like `Dec 3, 2026 at 2:30 PM` or `December 3 14:00`.
fn find_en_dates(
    text: &str,
    reference: DateTime<FixedOffset>,
    subject: &str,
    out: &mut Vec<TextSpanMatch>,
) {
    const MONTHS: &[(&str, u32)] = &[
        ("january", 1),
        ("february", 2),
        ("march", 3),
        ("april", 4),
        ("may", 5),
        ("june", 6),
        ("july", 7),
        ("august", 8),
        ("september", 9),
        ("october", 10),
        ("november", 11),
        ("december", 12),
        ("jan", 1),
        ("feb", 2),
        ("mar", 3),
        ("apr", 4),
        ("jun", 6),
        ("jul", 7),
        ("aug", 8),
        ("sep", 9),
        ("sept", 9),
        ("oct", 10),
        ("nov", 11),
        ("dec", 12),
    ];
    let lower = text.to_ascii_lowercase();
    let mut i = 0;
    while i < text.len() {
        if is_boundary_before(text, i) {
            let mut matched = None;
            for &(name, month_num) in MONTHS {
                if lower[i..].starts_with(name) {
                    let after_m = i + name.len();
                    let rest = &text[after_m..];
                    let rest = rest.strip_prefix('.').unwrap_or(rest);
                    let skipped = text.len() - rest.len();
                    let trimmed = rest.trim_start_matches(' ');
                    if trimmed.len() == rest.len() {
                        continue;
                    }
                    let day_start = skipped + (rest.len() - trimmed.len());
                    if let Some((d, d_len)) = take_u32(&text[day_start..], 2)
                        && (1..=31).contains(&d)
                    {
                        let mut end = day_start + d_len;
                        // Optional ordinal st/nd/rd/th
                        for ord in ["st", "nd", "rd", "th"] {
                            if lower[end..].starts_with(ord) {
                                end += 2;
                                break;
                            }
                        }
                        if !is_boundary_after(text, end) {
                            continue;
                        }
                        let mut year = reference.year();
                        let after_day = text[end..].trim_start_matches([',', ' ']);
                        let after_day_idx = text.len() - after_day.len();
                        if let Some((y, 4)) = take_u32(after_day, 4)
                            && (2020..=2100).contains(&y)
                            && is_boundary_after(text, after_day_idx + 4)
                        {
                            year = y as i32;
                            end = after_day_idx + 4;
                        }
                        if let Some(date) = NaiveDate::from_ymd_opt(year, month_num, d) {
                            let mut time = None;
                            if let Some((t, t_len)) = parse_hm_time(&text[end..]) {
                                time = Some(t);
                                end += t_len;
                            }
                            matched = Some((end, date, time));
                            break;
                        }
                    }
                }
            }
            if let Some((end, date, time)) = matched {
                let (start_at, all_day) = format_date_or_datetime(date, time, *reference.offset());
                out.push(TextSpanMatch {
                    start: i,
                    end,
                    kind: ActionItemKind::Event,
                    start_at: Some(start_at),
                    all_day,
                    matched_text: text[i..end].to_owned(),
                    suggested_title: subject.to_owned(),
                });
                i = end;
                continue;
            }
        }
        i += text[i..].chars().next().map_or(1, char::len_utf8);
    }
}

/// Matches Chinese relative dates like `明天上午10点`, `下周三 15:00`, `本周五`.
fn find_cn_relative_dates(
    text: &str,
    reference: DateTime<FixedOffset>,
    subject: &str,
    out: &mut Vec<TextSpanMatch>,
) {
    let base_date = reference.date_naive();
    let mut i = 0;
    while i < text.len() {
        let slice = &text[i..];
        let mut resolved: Option<(usize, NaiveDate)> = None;
        for (kw, delta) in [("大后天", 3i64), ("后天", 2), ("明天", 1), ("明日", 1)] {
            if slice.starts_with(kw) {
                resolved = Some((kw.len(), base_date + Duration::days(delta)));
                break;
            }
        }
        if resolved.is_none() {
            for (prefix, next_week) in [
                ("下周", true),
                ("下星期", true),
                ("本周", false),
                ("这周", false),
                ("周", false),
            ] {
                if let Some(rest) = slice.strip_prefix(prefix)
                    && let Some((wd, wd_len)) = parse_cn_weekday(rest)
                {
                    let date = resolve_weekday(base_date, wd, next_week);
                    resolved = Some((prefix.len() + wd_len, date));
                    break;
                }
            }
        }
        if let Some((kw_len, date)) = resolved {
            let mut end = i + kw_len;
            let mut time = None;
            if let Some((t, t_len)) = parse_hm_time(&text[end..]) {
                time = Some(t);
                end += t_len;
            }
            // Bare "周X" without "本/下" requires an attached time or deadline context to avoid noise.
            let is_bare_zhou = slice.starts_with('周') && time.is_none();
            if !is_bare_zhou {
                let (start_at, all_day) = format_date_or_datetime(date, time, *reference.offset());
                out.push(TextSpanMatch {
                    start: i,
                    end,
                    kind: ActionItemKind::Event,
                    start_at: Some(start_at),
                    all_day,
                    matched_text: text[i..end].to_owned(),
                    suggested_title: subject.to_owned(),
                });
                i = end;
                continue;
            }
        }
        i += slice.chars().next().map_or(1, char::len_utf8);
    }
}

fn parse_cn_weekday(s: &str) -> Option<(Weekday, usize)> {
    let ch = s.chars().next()?;
    let wd = match ch {
        '一' => Weekday::Mon,
        '二' => Weekday::Tue,
        '三' => Weekday::Wed,
        '四' => Weekday::Thu,
        '五' => Weekday::Fri,
        '六' => Weekday::Sat,
        '日' | '天' => Weekday::Sun,
        _ => return None,
    };
    Some((wd, ch.len_utf8()))
}

fn resolve_weekday(base: NaiveDate, target: Weekday, next_week: bool) -> NaiveDate {
    let cur = base.weekday().num_days_from_monday() as i64;
    let tgt = target.num_days_from_monday() as i64;
    let delta = if next_week {
        (7 - cur) + tgt
    } else if tgt >= cur {
        tgt - cur
    } else {
        (7 - cur) + tgt
    };
    base + Duration::days(delta)
}

/// Matches English relative dates like `tomorrow at 2pm`, `next Wednesday`, `by Friday`.
fn find_en_relative_dates(
    text: &str,
    reference: DateTime<FixedOffset>,
    subject: &str,
    out: &mut Vec<TextSpanMatch>,
) {
    const DAYS: &[(&str, Weekday)] = &[
        ("monday", Weekday::Mon),
        ("tuesday", Weekday::Tue),
        ("wednesday", Weekday::Wed),
        ("thursday", Weekday::Thu),
        ("friday", Weekday::Fri),
        ("saturday", Weekday::Sat),
        ("sunday", Weekday::Sun),
    ];
    let lower = text.to_ascii_lowercase();
    let base_date = reference.date_naive();
    let mut i = 0;
    while i < text.len() {
        if is_boundary_before(text, i) {
            let slice = &lower[i..];
            let mut resolved: Option<(usize, NaiveDate, ActionItemKind)> = None;
            if slice.starts_with("tomorrow") && is_boundary_after(text, i + 8) {
                resolved = Some((8, base_date + Duration::days(1), ActionItemKind::Event));
            } else {
                for (prefix, next_week, kind) in [
                    ("next ", true, ActionItemKind::Event),
                    ("this ", false, ActionItemKind::Event),
                    ("by ", false, ActionItemKind::Task),
                    ("due ", false, ActionItemKind::Task),
                ] {
                    if let Some(rest) = slice.strip_prefix(prefix) {
                        for &(day_name, wd) in DAYS {
                            if rest.starts_with(day_name)
                                && is_boundary_after(text, i + prefix.len() + day_name.len())
                            {
                                let date = resolve_weekday(base_date, wd, next_week);
                                resolved = Some((prefix.len() + day_name.len(), date, kind));
                                break;
                            }
                        }
                    }
                    if resolved.is_some() {
                        break;
                    }
                }
            }
            if let Some((len, date, kind)) = resolved {
                let mut end = i + len;
                let mut time = None;
                if let Some((t, t_len)) = parse_hm_time(&text[end..]) {
                    time = Some(t);
                    end += t_len;
                }
                let (start_at, all_day) = format_date_or_datetime(date, time, *reference.offset());
                out.push(TextSpanMatch {
                    start: i,
                    end,
                    kind,
                    start_at: Some(start_at),
                    all_day,
                    matched_text: text[i..end].to_owned(),
                    suggested_title: subject.to_owned(),
                });
                i = end;
                continue;
            }
        }
        i += text[i..].chars().next().map_or(1, char::len_utf8);
    }
}

/// Matches high-signal dateless task phrases while excluding common courtesies.
fn find_high_signal_tasks(text: &str, out: &mut Vec<TextSpanMatch>) {
    const CN_COURTESIES: &[&str] = &[
        "请查收",
        "请知悉",
        "请见附件",
        "请看附件",
        "麻烦看一下",
        "麻烦看下",
        "请多关照",
        "请谅解",
        "请放心",
    ];
    const EN_COURTESIES: &[&str] = &[
        "please let me know",
        "please find attached",
        "please see attached",
        "please feel free",
        "please do not hesitate",
        "please note that",
    ];
    const CN_TRIGGERS: &[&str] = &[
        "待办：",
        "待办:",
        "行动项：",
        "行动项:",
        "需要你签署",
        "需要您签署",
        "需要你审批",
        "需要您审批",
        "需要你提交",
        "需要您提交",
        "需要你付款",
        "需要您付款",
        "需要你盖章",
        "需要您盖章",
        "请签署",
        "请审批",
        "请提交",
        "请付款",
        "请支付",
        "请盖章",
        "请尽快签署",
        "请尽快审批",
        "请尽快提交",
        "请尽快付款",
        "请尽快确认",
        "请确认合同",
        "请回复确认",
    ];
    const EN_TRIGGERS: &[&str] = &[
        "todo:",
        "action item:",
        "action required:",
        "please sign ",
        "please submit ",
        "please approve ",
        "please pay ",
        "please review ",
        "please complete ",
        "please confirm ",
    ];

    let lower = text.to_ascii_lowercase();
    for trigger in CN_TRIGGERS {
        let mut search_from = 0;
        while let Some(pos) = text[search_from..].find(trigger) {
            let start = search_from + pos;
            let end = clause_end(text, start, 60);
            let clause = text[start..end].trim();
            if !CN_COURTESIES.iter().any(|c| clause.contains(c)) && clause.chars().count() >= 4 {
                out.push(TextSpanMatch {
                    start,
                    end: start + clause.len(),
                    kind: ActionItemKind::Task,
                    start_at: None,
                    all_day: false,
                    matched_text: clause.to_owned(),
                    suggested_title: clean_text(clause, MAX_TITLE_CHARS, false),
                });
            }
            search_from = end.max(start + trigger.len());
        }
    }

    for trigger in EN_TRIGGERS {
        let mut search_from = 0;
        while let Some(pos) = lower[search_from..].find(trigger) {
            let start = search_from + pos;
            if !is_boundary_before(text, start) {
                search_from = start + trigger.len();
                continue;
            }
            let end = clause_end(text, start, 80);
            let clause = text[start..end].trim();
            let clause_lower = clause.to_ascii_lowercase();
            if !EN_COURTESIES.iter().any(|c| clause_lower.starts_with(c))
                && clause.len() >= trigger.len() + 2
            {
                out.push(TextSpanMatch {
                    start,
                    end: start + clause.len(),
                    kind: ActionItemKind::Task,
                    start_at: None,
                    all_day: false,
                    matched_text: clause.to_owned(),
                    suggested_title: clean_text(clause, MAX_TITLE_CHARS, false),
                });
            }
            search_from = end.max(start + trigger.len());
        }
    }
}

fn clause_end(text: &str, start: usize, max_chars: usize) -> usize {
    let mut chars = 0usize;
    let mut end = start;
    for (offset, ch) in text[start..].char_indices() {
        if matches!(ch, '。' | '！' | '？' | '\n' | '\r' | ';' | '；' | '.') && offset > 3 {
            break;
        }
        chars += 1;
        end = start + offset + ch.len_utf8();
        if chars >= max_chars {
            break;
        }
    }
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_cn_and_en_dates_with_reference_time() {
        // Friday 2026-11-27 10:00 +08:00
        let (html, items) = annotate_html(
            "<p>我们明天上午10点开会，另外 Dec 3, 2026 at 2:30 PM 有评审。</p>",
            "项目沟通",
            Some("2026-11-27T10:00:00+08:00"),
            15,
        );
        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0].start_at.as_deref(),
            Some("2026-11-28T10:00:00+08:00")
        );
        assert_eq!(
            items[1].start_at.as_deref(),
            Some("2026-12-03T14:30:00+08:00")
        );
        assert!(html.contains("data-mailx-action=\"0\""));
        assert!(html.contains("data-mailx-action=\"1\""));
    }

    #[test]
    fn skips_links_code_and_folded_quotes() {
        let (html, items) = annotate_html(
            "<a href=\"x\">2026-12-03</a><code>2026-12-04</code><details><summary>Quote</summary><p>2026-12-05</p></details><p>2026-12-06 15:00</p>",
            "Test",
            Some("2026-11-27T10:00:00+08:00"),
            15,
        );
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].matched_text.as_deref(), Some("2026-12-06 15:00"));
        assert!(html.contains("data-mailx-action=\"0\""));
    }

    #[test]
    fn filters_courtesies_and_otp_expiry_noise() {
        let (_, otp) = annotate_html(
            "<p>您的验证码是 123456，10分钟内有效，明天过期前请勿泄露。</p>",
            "验证码",
            Some("2026-11-27T10:00:00+08:00"),
            15,
        );
        assert!(otp.is_empty());

        let (_, courtesy) = annotate_html(
            "<p>附件已发送，请查收。Please let me know if you have any questions.</p>",
            "资料",
            Some("2026-11-27T10:00:00+08:00"),
            15,
        );
        assert!(courtesy.is_empty());

        let (_, task) = annotate_html(
            "<p>请尽快签署年度服务协议并回传。Please sign the NDA before onboarding.</p>",
            "合同",
            Some("2026-11-27T10:00:00+08:00"),
            15,
        );
        assert_eq!(task.len(), 2);
        assert_eq!(task[0].kind, ActionItemKind::Task);
        assert_eq!(task[1].kind, ActionItemKind::Task);
    }
}
