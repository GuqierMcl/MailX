//! Experimental, local verification-code recognition. This module has no network,
//! storage, notification or clipboard side effects. A match is a text heuristic,
//! not evidence of sender authenticity, freshness or a pending login request.

use std::{fmt, ops::Range};

use serde::Serialize;

mod mime;
pub use mime::{MAX_EML_BYTES, detect_eml};

pub const MAX_SUBJECT_BYTES: usize = 4096;
pub const MAX_TEXT_BYTES: usize = 64 * 1024;
const MAX_CANDIDATES: usize = 16;
const CONTEXT_BYTES: usize = 96;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    Subject,
    Body,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Rule {
    KeywordBeforeCode,
    CodeBeforeKeyword,
    SubjectWithStandaloneCode,
}

/// Controlled matching metadata; no surrounding email text is retained.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    pub source: Source,
    pub rule: Rule,
    pub keyword: &'static str,
}

/// Serialization intentionally exposes the code for an explicit caller (such
/// as the local probe). Debug formatting never includes its value.
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct Candidate {
    pub code: String,
    pub evidence: Vec<Evidence>,
}

impl fmt::Debug for Candidate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Candidate")
            .field("code", &"[redacted]")
            .field("evidence", &self.evidence)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum Detection {
    Found { candidate: Candidate },
    Ambiguous { candidates: Vec<Candidate> },
    NotFound,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetectionError {
    InputTooLarge,
    InvalidMessage,
}

impl fmt::Display for DetectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InputTooLarge => "verification-code input exceeds the processing budget",
            Self::InvalidMessage => "verification-code input is not a supported message",
        })
    }
}

impl std::error::Error for DetectionError {}

const KEYWORDS: &[&str] = &[
    "verification code",
    "security code",
    "authentication code",
    "confirmation code",
    "login code",
    "sign-in code",
    "sign in code",
    "one-time password",
    "one time password",
    "one-time code",
    "one time code",
    "otp",
    "验证码",
    "驗證碼",
    "校验码",
    "校驗碼",
    "动态密码",
    "動態密碼",
    "动态码",
    "動態碼",
];

const SUFFIXES: &[&str] = &[
    "to sign in",
    "to log in",
    "to verify your email",
    "to verify your account",
];

/// Recognize a unique 4–8 character ASCII digit or mixed letter/digit code in
/// decoded subject and plain body text. Case and leading zeroes are preserved.
/// Explicit nearby verification language is required. Conflicting candidates
/// are returned as ambiguous, never ranked into a winner.
///
/// Reply/forward subjects and common plain-text quotations are ignored. Pass
/// HTML through `detect_eml` instead of flattening it without quote boundaries.
/// Oversized inputs (or more than 16 distinct candidates) fail rather than
/// silently scanning a prefix that might hide a conflicting code.
pub fn detect(subject: &str, body: &str) -> Result<Detection, DetectionError> {
    if subject.len() > MAX_SUBJECT_BYTES || body.len() > MAX_TEXT_BYTES {
        return Err(DetectionError::InputTooLarge);
    }
    let subject = if reply_subject(subject) { "" } else { subject };
    let subject = normalize(subject);
    let subject_keyword = keyword_in(&subject.to_ascii_lowercase());
    let body = unquoted(body);
    let mut candidates = Vec::new();
    scan(&subject, Source::Subject, None, &mut candidates)?;
    scan(&body, Source::Body, subject_keyword, &mut candidates)?;
    Ok(match candidates.len() {
        0 => Detection::NotFound,
        1 => Detection::Found {
            candidate: candidates.remove(0),
        },
        _ => Detection::Ambiguous { candidates },
    })
}

fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .map(|c| {
            if c.is_whitespace() && c != '\n' {
                ' '
            } else {
                c
            }
        })
        .collect()
}

fn reply_subject(subject: &str) -> bool {
    let subject = subject.trim_start().to_ascii_lowercase();
    [
        "re:",
        "fw:",
        "fwd:",
        "re：",
        "fw：",
        "fwd：",
        "回复:",
        "回复：",
        "回覆:",
        "回覆：",
        "转发:",
        "转发：",
        "轉寄:",
        "轉寄：",
    ]
    .iter()
    .any(|prefix| subject.starts_with(prefix))
}

fn unquoted(body: &str) -> String {
    let body = normalize(body);
    let mut text = String::new();
    for line in body.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        if (lower.starts_with("on ") && lower.ends_with("wrote:"))
            || (trimmed.starts_with('在')
                && (trimmed.ends_with("写道：") || trimmed.ends_with("寫道：")))
            || lower.contains("-----original message-----")
            || lower.contains("-----forwarded message-----")
            || lower.contains("---------- forwarded message ---------")
            || trimmed.contains("-----原始邮件-----")
        {
            break;
        }
        // A newline is retained even when the quoted line is skipped, so two
        // unrelated lines cannot become one token.
        if !trimmed.starts_with('>') {
            text.push_str(line);
        }
        text.push('\n');
    }
    text
}

fn scan(
    text: &str,
    source: Source,
    subject_keyword: Option<&'static str>,
    candidates: &mut Vec<Candidate>,
) -> Result<(), DetectionError> {
    let lower = text.to_ascii_lowercase();
    let network = network_tokens(text);
    let mut network_index = 0;
    let first_line = first_code_line(text);
    let mut previous: Option<(usize, Vec<Evidence>)> = None;
    let mut start = None;
    for (i, c) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        if c.is_ascii_alphanumeric() {
            start.get_or_insert(i);
        } else if let Some(begin) = start.take() {
            let range = begin..i;
            let code = &text[range.clone()];
            while network
                .get(network_index)
                .is_some_and(|token| token.end <= begin)
            {
                network_index += 1;
            }
            if !(4..=8).contains(&code.len())
                || !code.bytes().any(|b| b.is_ascii_digit())
                || network
                    .get(network_index)
                    .is_some_and(|token| token.start <= begin)
                || !whole_code(text, &range)
            {
                continue;
            }
            let mut evidence = evidence(&lower, &range, source, subject_keyword);
            if evidence.is_empty()
                && let Some((end, prior)) = &previous
                && list_separator(&lower[*end..begin])
            {
                evidence.clone_from(prior);
            }
            if evidence.is_empty()
                && code.bytes().all(|b| b.is_ascii_digit())
                && let Some(keyword) = subject_keyword
                && first_line == range
            {
                evidence.push(Evidence {
                    source,
                    rule: Rule::SubjectWithStandaloneCode,
                    keyword,
                });
            }
            if evidence.is_empty() {
                continue;
            }
            previous = Some((range.end, evidence.clone()));
            if let Some(existing) = candidates
                .iter_mut()
                .find(|candidate| candidate.code == code)
            {
                for item in evidence {
                    if !existing.evidence.contains(&item) {
                        existing.evidence.push(item);
                    }
                }
            } else {
                if candidates.len() == MAX_CANDIDATES {
                    return Err(DetectionError::InputTooLarge);
                }
                candidates.push(Candidate {
                    code: code.to_owned(),
                    evidence,
                });
            }
        }
    }
    Ok(())
}

fn evidence(
    lower: &str,
    range: &Range<usize>,
    source: Source,
    subject_keyword: Option<&'static str>,
) -> Vec<Evidence> {
    let before_start = lower.ceil_char_boundary(range.start.saturating_sub(CONTEXT_BYTES));
    let after_end = lower.floor_char_boundary((range.end + CONTEXT_BYTES).min(lower.len()));
    let before = &lower[before_start..range.start];
    let after = &lower[range.end..after_end];
    let mut evidence = Vec::new();
    for keyword in KEYWORDS
        .iter()
        .copied()
        .chain(subject_keyword.map(|_| "code"))
    {
        if let Some(i) = before.rfind(keyword)
            && keyword_boundary(before, i, keyword)
            && !old_context(&before[..i])
            && bridge(&before[i + keyword.len()..])
        {
            evidence.push(Evidence {
                source,
                rule: Rule::KeywordBeforeCode,
                keyword,
            });
        }
        if let Some(i) = after.find(keyword)
            && keyword_boundary(after, i, keyword)
            && bridge(&after[..i])
            && !old_context(before)
        {
            evidence.push(Evidence {
                source,
                rule: Rule::CodeBeforeKeyword,
                keyword,
            });
        }
    }
    for &keyword in SUFFIXES {
        if let Some(i) = after.find(keyword)
            && keyword_boundary(after, i, keyword)
            && bridge(&after[..i])
            && !old_context(before)
        {
            evidence.push(Evidence {
                source,
                rule: Rule::CodeBeforeKeyword,
                keyword,
            });
        }
    }
    evidence
}

// The gap is a small grammar, not merely a distance: "expires in 2026" and
// "order number 123456" must not become codes next to verification language.
fn bridge(gap: &str) -> bool {
    if gap.len() > 48 || gap.matches('\n').count() > 2 {
        return false;
    }
    let words = gap.split(|c: char| c.is_whitespace() || ":：=,，-–—()（）[]【】\"'`*".contains(c));
    words.filter(|word| !word.is_empty()).all(|word| {
        [
            "is",
            "your",
            "the",
            "this",
            "use",
            "enter",
            "below",
            "following",
            "为",
            "為",
            "是",
            "如下",
            "请使用",
            "請使用",
            "请输入",
            "請輸入",
        ]
        .contains(&word)
    })
}

fn old_context(prefix: &str) -> bool {
    let last = prefix
        .rsplit(['\n', '.', '。', '!', '！', ';', '；'])
        .next()
        .unwrap_or_default();
    last.split(|c: char| !c.is_ascii_alphabetic())
        .any(|word| ["old", "previous", "expired", "invalid", "example"].contains(&word))
        || [
            "旧",
            "舊",
            "上次",
            "过期",
            "過期",
            "示例",
            "do not use",
            "don't use",
        ]
        .iter()
        .any(|word| last.contains(word))
}

fn keyword_in(lower: &str) -> Option<&'static str> {
    KEYWORDS.iter().copied().find(|keyword| {
        lower
            .match_indices(keyword)
            .any(|(i, _)| keyword_boundary(lower, i, keyword) && !old_context(&lower[..i]))
    })
}

fn keyword_boundary(text: &str, start: usize, keyword: &str) -> bool {
    if !keyword.is_ascii() {
        return true;
    }
    let word = |c: char| c.is_alphanumeric() || c == '_';
    !text[..start].chars().next_back().is_some_and(word)
        && !text[start + keyword.len()..]
            .chars()
            .next()
            .is_some_and(word)
}

fn first_code_line(text: &str) -> Range<usize> {
    let trimmed = text.trim_start();
    let start = text.len() - trimmed.len();
    let line = trimmed.split('\n').next().unwrap_or_default().trim_end();
    start..start + line.len()
}

fn list_separator(gap: &str) -> bool {
    gap.len() <= 16 && ["or", "and", "或", "和", "、", ",", "，"].contains(&gap.trim())
}

// Find URL/address tokens once, not once per candidate: a long token with
// thousands of digit runs must still take linear time.
fn network_tokens(text: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = None;
    for (i, c) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        if !c.is_whitespace() {
            start.get_or_insert(i);
        } else if let Some(begin) = start.take() {
            let token = &text[begin..i];
            if token.contains("://")
                || token.contains('@')
                || token
                    .get(..4)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("www."))
            {
                ranges.push(begin..i);
            }
        }
    }
    ranges
}

fn whole_code(text: &str, range: &Range<usize>) -> bool {
    let before = &text[..range.start];
    let after = &text[range.end..];
    let han = |c: char| matches!(c, '\u{3400}'..='\u{9fff}' | '\u{f900}'..='\u{faff}');
    let attached = |c: char| {
        c == '_'
            || c == '@'
            || c == '/'
            || c == '\\'
            || c == '\u{fffd}'
            || c.is_ascii_control() && !c.is_whitespace()
            || c.is_alphanumeric() && !han(c)
            || (!c.is_ascii()
                && !han(c)
                && !c.is_whitespace()
                && !"：，。！？；（）【】“”‘’、".contains(c))
    };
    if before.chars().next_back().is_some_and(attached)
        || after.chars().next().is_some_and(attached)
    {
        return false;
    }
    let prev = before.trim_end();
    let next = after.trim_start();
    if prev.ends_with(['$', '€', '£', '¥', '￥', '+']) || next.starts_with(['%', '％']) {
        return false;
    }
    // Reject pieces of grouped numbers, phone numbers, dates, decimals and
    // hyphenated codes; these formats are deliberately outside the first pass.
    if separated_digit(prev.chars().rev()) || separated_digit(next.chars()) {
        return false;
    }
    true
}

fn separated_digit(mut chars: impl Iterator<Item = char>) -> bool {
    let Some(first) = chars.next() else {
        return false;
    };
    if first.is_ascii_digit() {
        return true;
    }
    if first.is_ascii_alphabetic() {
        let word: String = std::iter::once(first)
            .chain(chars)
            .take_while(|c| c.is_ascii_alphanumeric())
            .take(9)
            .collect();
        return (4..=8).contains(&word.len()) && word.bytes().any(|b| b.is_ascii_digit());
    }
    if first == ',' || first == '，' {
        let word: String = chars
            .skip_while(|c| c.is_whitespace())
            .take_while(|c| c.is_ascii_alphanumeric())
            .take(9)
            .collect();
        // Two code-sized values can form an explicit comma-separated list;
        // the scanner carries the same evidence to the second, making it
        // ambiguous. Short ASCII numeric groups are not partial codes.
        if (4..=8).contains(&word.len()) && word.bytes().any(|b| b.is_ascii_digit()) {
            return false;
        }
        return first == ',' && word.bytes().next().is_some_and(|b| b.is_ascii_digit());
    }
    if "-–—/".contains(first) {
        return chars
            .find(|c| !c.is_whitespace())
            .is_some_and(|c| c.is_ascii_alphanumeric());
    }
    if ".:".contains(first) {
        return chars.next().is_some_and(|c| c.is_ascii_digit());
    }
    false
}

#[cfg(test)]
mod tests;
