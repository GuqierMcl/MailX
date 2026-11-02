//! Audit sink for tool calls. Records carry identifiers and error codes only;
//! arguments, search text and mail content are never part of a record.

use mailx_core::tools::{ToolAudit, ToolCallRecord};

/// Writes one line per call to standard error, the desktop host's only log
/// channel. Free-form fields (client name) are sanitized upstream.
#[derive(Clone, Copy, Debug, Default)]
pub struct StderrAudit;

impl ToolAudit for StderrAudit {
    fn record(&self, record: &ToolCallRecord) {
        eprintln!("{}", format_record(record));
    }
}

/// Strips control characters and caps the length of text that a remote peer
/// chose, so it cannot forge log lines or flood the UI.
pub(crate) fn sanitize_label(raw: &str, max_chars: usize) -> String {
    raw.chars()
        .filter(|ch| !ch.is_control())
        .take(max_chars)
        .collect::<String>()
        .trim()
        .to_owned()
}

pub(crate) fn format_record(record: &ToolCallRecord) -> String {
    let outcome = match record.error {
        None => "ok".to_owned(),
        Some(code) => format!("error:{code:?}"),
    };
    format!(
        "[mailx-mcp] tool={} principal={}:{} client={} request={} resources={} outcome={}",
        sanitize_label(&record.tool_id, 64),
        record.principal_kind,
        record.principal_id,
        record
            .client_name
            .as_deref()
            .map_or_else(|| "-".to_owned(), |name| sanitize_label(name, 64)),
        record.request_id,
        record.resource_ids.len(),
        outcome,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_lose_control_characters_and_length() {
        assert_eq!(sanitize_label("a\nb\u{1b}[0m", 64), "ab[0m");
        assert_eq!(sanitize_label(&"x".repeat(100), 10).len(), 10);
        assert_eq!(sanitize_label("  \n ", 10), "");
    }

    #[test]
    fn records_hold_no_arguments_and_one_line() {
        let line = format_record(&ToolCallRecord {
            request_id: "c:1".into(),
            principal_kind: "localMcp",
            principal_id: "conn".into(),
            client_name: Some("evil\nclient".into()),
            tool_id: "search_emails".into(),
            resource_ids: vec!["a".into(), "b".into()],
            error: None,
        });
        assert!(!line.contains('\n'));
        assert!(line.contains("tool=search_emails"));
        assert!(line.contains("resources=2"));
        assert!(line.contains("outcome=ok"));
    }
}
