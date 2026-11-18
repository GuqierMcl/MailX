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
            reasoning: None,
        },
        ChatMessage::ToolResult {
            call_id: "1".into(),
            content: "body".into(),
            is_error: true,
        },
        ChatMessage::Assistant {
            text: "done".into(),
            tool_calls: Vec::new(),
            reasoning: None,
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
            reasoning: None,
        },
        ChatMessage::ToolResult {
            call_id: "1".into(),
            content: "r".into(),
            is_error: false,
        },
        ChatMessage::Assistant {
            text: "!".into(),
            tool_calls: vec![call("2")],
            reasoning: None,
        },
        ChatMessage::ToolResult {
            call_id: "2".into(),
            content: "r".into(),
            is_error: false,
        },
        ChatMessage::Assistant {
            text: "end".into(),
            tool_calls: Vec::new(),
            reasoning: None,
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

fn reasoning(text: &str, spans: Vec<crate::ai::reasoning::ReasoningSpan>) -> AssistantReasoning {
    use crate::ai::reasoning::{ReasoningEcho, ReasoningField, ReasoningOrigin};
    AssistantReasoning {
        origin: ReasoningOrigin {
            protocol: crate::AiProtocol::OpenaiCompatible,
            connection_id: "00000000-0000-4000-8000-000000000001".into(),
            base_url: "https://ai.example/v1".into(),
            model: "m".into(),
        },
        echo: Some(ReasoningEcho::Field {
            field: ReasoningField::ReasoningContent,
            text: text.into(),
        }),
        spans,
    }
}

fn span(bytes: usize, duration_ms: u64, text_at: usize) -> crate::ai::reasoning::ReasoningSpan {
    crate::ai::reasoning::ReasoningSpan {
        bytes,
        duration_ms,
        text_at,
        tools_before: 0,
    }
}

#[test]
fn reasoning_round_trips_and_old_rows_without_it_still_load() {
    let history = [
        ChatMessage::User("hi".into()),
        ChatMessage::Assistant {
            text: "calling".into(),
            tool_calls: vec![ToolCallRequest {
                id: "1".into(),
                name: "read_email".into(),
                arguments: json!({"emailId": "e"}),
            }],
            reasoning: Some(reasoning("想一想", vec![span(9, 1500, 0)])),
        },
    ];
    let rows: Vec<StoredRow> = history
        .iter()
        .map(|m| stored(message_row(m, None, "t")))
        .collect();
    assert!(rows[1].content.contains("reasoning_content"));
    assert_eq!(load(&rows).unwrap().messages, history);

    // A row written before reasoning was kept.
    let old = StoredRow {
        role: "assistant".into(),
        content: r#"{"type":"assistant","text":"old","toolCalls":[]}"#.into(),
        created_at: "t".into(),
    };
    assert_eq!(
        load(&[old]).unwrap().messages,
        [ChatMessage::Assistant {
            text: "old".into(),
            tool_calls: Vec::new(),
            reasoning: None,
        }]
    );
    // A message without reasoning writes no field.
    let plain = message_row(
        &ChatMessage::Assistant {
            text: "x".into(),
            tool_calls: Vec::new(),
            reasoning: None,
        },
        None,
        "t",
    );
    assert!(!plain.content.contains("reasoning"), "{}", plain.content);
}

#[test]
fn saved_turns_place_each_thinking_stretch_among_text_and_tools() {
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
        // Thinks, writes "a😀b", calls a tool.
        ChatMessage::Assistant {
            text: "a\u{1F600}b".into(),
            tool_calls: vec![call("1")],
            reasoning: Some(reasoning("first", vec![span(5, 2000, 0)])),
        },
        ChatMessage::ToolResult {
            call_id: "1".into(),
            content: "r".into(),
            is_error: false,
        },
        // Writes "!", thinks again, then answers.
        ChatMessage::Assistant {
            text: "!end".into(),
            tool_calls: Vec::new(),
            reasoning: Some(reasoning("second", vec![span(6, 300, 1)])),
        },
    ];
    let rows: Vec<StoredRow> = messages
        .iter()
        .enumerate()
        .map(|(i, m)| {
            stored(message_row(
                m,
                (i == 2).then(|| activity("1")).as_ref(),
                "t",
            ))
        })
        .collect();
    let shown = turns(&rows);
    let ChatTurn::Assistant { reasoning, .. } = &shown[1] else {
        panic!("{shown:?}");
    };
    assert_eq!(
        reasoning,
        &[
            ChatTurnReasoning {
                text: "first".into(),
                duration_ms: 2000,
                text_offset: 0,
                tools_before: 0,
            },
            ChatTurnReasoning {
                text: "second".into(),
                duration_ms: 300,
                // "a😀b" is four units, then "!".
                text_offset: 5,
                tools_before: 1,
            },
        ]
    );
}
