use serde_json::json;

use super::*;
use crate::assistant::types::ToolUndo;

fn stored(row: NewRow) -> StoredRow {
    StoredRow {
        role: row.role.to_owned(),
        content: row.content,
        created_at: row.created_at,
    }
}

#[test]
fn titles_are_one_short_line() {
    assert_eq!(title("  hello \n\t world  "), "hello world");
    let long = "word ".repeat(40);
    let cut = title(&long);
    assert!(cut.chars().count() <= TITLE_CHARS + 1, "{cut}");
    assert!(cut.ends_with('\u{2026}') && !cut.contains("  "));
    assert_eq!(title(&"x".repeat(TITLE_CHARS)).chars().count(), TITLE_CHARS);
    assert_eq!(title(&"你".repeat(100)).chars().count(), TITLE_CHARS + 1);
}

#[test]
fn undo_data_is_never_written() {
    let record = ToolRecord {
        activity: ToolActivity {
            call_id: "c".into(),
            name: "archive_emails".into(),
            status: crate::assistant::ToolStatus::Done,
            summary: ToolActivitySummary::Changed {
                count: 1,
                email_ids: vec!["e".into()],
                undo: Some(ToolUndo::Archive {
                    token: "secret-token".into(),
                }),
            },
        },
        confirmation: None,
    };
    let row = message_row(
        &ChatMessage::ToolResult {
            call_id: "c".into(),
            content: "r".into(),
            is_error: false,
        },
        Some(&record),
        "t",
    );
    assert!(!row.content.contains("secret-token"));
    assert!(row.content.contains("archive_emails"));
}

#[test]
fn rows_round_trip_into_model_history() {
    let history = [
        ChatMessage::User("hi".into()),
        ChatMessage::Assistant {
            text: "calling".into(),
            tool_calls: vec![ToolCallRequest {
                id: "1".into(),
                name: "read_email".into(),
                arguments: json!({"emailId": "e"}),
            }],
        },
        ChatMessage::ToolResult {
            call_id: "1".into(),
            content: "body".into(),
            is_error: true,
        },
        ChatMessage::Assistant {
            text: "done".into(),
            tool_calls: Vec::new(),
        },
    ];
    let mut rows: Vec<StoredRow> = history
        .iter()
        .map(|m| stored(message_row(m, None, "t")))
        .collect();
    rows.push(stored(end_row(&End::finished(FinishedReason::Stop), "t")));
    let loaded = load(&rows).unwrap();
    assert_eq!(loaded.messages, history);
    assert_eq!(loaded.stamps.len(), 4);
    assert_eq!(loaded.rows, 5, "the closing row counts as a row");

    rows[1].content = "not json".into();
    assert_eq!(
        load(&rows).err().map(|e| e.code),
        Some(ErrorCode::StorageIncompatible)
    );
}

#[test]
fn text_offsets_count_utf16_units_like_the_interface() {
    let call = |id: &str| ToolCallRequest {
        id: id.into(),
        name: "search_emails".into(),
        arguments: json!({}),
    };
    let activity = |id: &str| ToolRecord {
        activity: ToolActivity {
            call_id: id.into(),
            name: "search_emails".into(),
            status: crate::assistant::ToolStatus::Done,
            summary: ToolActivitySummary::Found { count: 1 },
        },
        confirmation: None,
    };
    let messages = [
        ChatMessage::User("q".into()),
        ChatMessage::Assistant {
            text: "a\u{1F600}b".into(), // an emoji takes two units
            tool_calls: vec![call("1")],
        },
        ChatMessage::ToolResult {
            call_id: "1".into(),
            content: "r".into(),
            is_error: false,
        },
        ChatMessage::Assistant {
            text: "!".into(),
            tool_calls: vec![call("2")],
        },
        ChatMessage::ToolResult {
            call_id: "2".into(),
            content: "r".into(),
            is_error: false,
        },
        ChatMessage::Assistant {
            text: "end".into(),
            tool_calls: Vec::new(),
        },
    ];
    let records = [(2, activity("1")), (4, activity("2"))];
    let rows: Vec<StoredRow> = messages
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let record = records.iter().find(|(at, _)| *at == i).map(|(_, r)| r);
            stored(message_row(m, record, "t"))
        })
        .collect();
    let shown = turns(&rows);
    let ChatTurn::Assistant { text, tools, .. } = &shown[1] else {
        panic!("{shown:?}");
    };
    assert_eq!(text, "a\u{1F600}b!end");
    assert_eq!(
        tools.iter().map(|t| t.text_offset).collect::<Vec<_>>(),
        [4, 5]
    );
    // No closing row: the reply was cut off.
    assert!(matches!(
        &shown[1],
        ChatTurn::Assistant {
            state: TurnState::Canceled,
            ..
        }
    ));
}

#[test]
fn accounts_come_from_inputs_and_results_and_only_real_ids_count() {
    let a = "11111111-1111-4111-8111-111111111111";
    let b = "22222222-2222-4222-8222-222222222222";
    let mut found = BTreeSet::new();
    touched_accounts(
        &json!({"accountId": a, "accountIds": [b, "nope"], "query": "x"}),
        &json!({"emails": [{"id": "e", "accountId": a.to_uppercase()}, {"accountId": 5}]}),
        &mut found,
    );
    assert_eq!(found.into_iter().collect::<Vec<_>>(), [a, b]);
}
