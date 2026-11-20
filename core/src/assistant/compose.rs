//! Writing help in the composer (`docs/architecture/ai-chat-ui.md` section
//! 5.3). The interface names the draft and sends the user's own words; the
//! core reads the draft's headers and the email it answers itself, within the
//! assistant's accounts, and everything reaches the model as untrusted data.
//! Nothing here writes to the draft: the answer is Markdown the editor shows
//! as a preview until the user takes it.
use std::fmt::Write as _;

use serde_json::{Value, json};

use super::{
    Inner, agent,
    types::{ComposeTask, MAX_COMPOSE_INSTRUCTION_CHARS, MAX_COMPOSE_TEXT_CHARS, RewriteStyle},
};
use crate::{
    AppError, DraftKind, ErrorCode,
    settings::AssistantSettings,
    tools::{Principal, RequestContext},
    translate::{languages, provider::ai::language_name},
};

const MAX_ID_CHARS: usize = 128;
/// Body text of the email a draft answers.
const SOURCE_BODY_CHARS: usize = 8_000;
const MAX_HEADER_CHARS: usize = 300;

fn text_ok(text: &str, max: usize) -> bool {
    !text.trim().is_empty() && text.chars().count() <= max
}

fn code_ok(code: &str) -> bool {
    languages::is_target(code) && language_name(code).is_some()
}

/// Whether the request is well formed; the draft itself is checked when read.
pub(super) fn valid(draft_id: &str, task: &ComposeTask) -> Result<(), AppError> {
    if draft_id.is_empty() || draft_id.chars().count() > MAX_ID_CHARS {
        return Err(AppError::validation("draftId"));
    }
    match task {
        ComposeTask::Draft {
            instruction,
            language,
        } => {
            if !text_ok(instruction, MAX_COMPOSE_INSTRUCTION_CHARS) {
                return Err(AppError::validation("task.instruction"));
            }
            if language.as_deref().is_some_and(|code| !code_ok(code)) {
                return Err(AppError::validation("task.language"));
            }
        }
        ComposeTask::Rewrite { text, style } => {
            if !text_ok(text, MAX_COMPOSE_TEXT_CHARS) {
                return Err(AppError::validation("task.text"));
            }
            match style {
                RewriteStyle::Translate { code } if !code_ok(code) => {
                    return Err(AppError::validation("task.style"));
                }
                RewriteStyle::Custom { instruction }
                    if !text_ok(instruction, MAX_COMPOSE_INSTRUCTION_CHARS) =>
                {
                    return Err(AppError::validation("task.style"));
                }
                _ => {}
            }
        }
        ComposeTask::Proofread { text } => {
            if !text_ok(text, MAX_COMPOSE_TEXT_CHARS) {
                return Err(AppError::validation("task.text"));
            }
        }
    }
    Ok(())
}

fn flat(text: &str, max: usize) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(max)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Text inside a section cannot close it early.
fn fenced(text: &str, tag: &str) -> String {
    text.replace(&format!("</{tag}>"), &format!("</ {tag}>"))
}

/// Reads the draft and what it answers on a blocking thread and builds the
/// model's input. `NotFound` when the draft is gone or its account is outside
/// the assistant's.
pub(super) async fn input(
    inner: &std::sync::Arc<Inner>,
    settings: &AssistantSettings,
    draft_id: &str,
    task: &ComposeTask,
) -> Result<String, AppError> {
    let inner = inner.clone();
    let settings = settings.clone();
    let draft_id = draft_id.to_owned();
    let task = task.clone();
    tokio::task::spawn_blocking(move || read(&inner, &settings, &draft_id, &task))
        .await
        .map_err(|_| AppError::new(ErrorCode::Internal))?
}

fn read(
    inner: &Inner,
    settings: &AssistantSettings,
    draft_id: &str,
    task: &ComposeTask,
) -> Result<String, AppError> {
    let grant =
        agent::grant_for(inner, settings).ok_or_else(|| AppError::new(ErrorCode::Internal))?;
    let draft = inner.core.get_draft(draft_id)?;
    if !grant.accounts.permits(&draft.account_id) {
        return Err(AppError::new(ErrorCode::NotFound));
    }
    let sender = inner
        .core
        .account_identities()?
        .into_iter()
        .find(|account| account.id == draft.account_id);
    // The email answered, read through the tool layer so the scope applies;
    // missing, out of scope or not downloaded leaves it out.
    let source = match (&draft.kind, &draft.source_email_id) {
        (DraftKind::Reply | DraftKind::ReplyAll | DraftKind::Forward, Some(id))
            if matches!(task, ComposeTask::Draft { .. }) =>
        {
            let context = RequestContext::new(
                Principal::BuiltinAgent {
                    run_id: "compose-assist".to_owned(),
                },
                grant,
                "compose-assist".to_owned(),
            );
            inner
                .tools
                .call(
                    &context,
                    "mail_read",
                    json!({ "emailId": id, "maxChars": SOURCE_BODY_CHARS }),
                )
                .ok()
        }
        _ => None,
    };
    Ok(build(
        &draft,
        sender
            .as_ref()
            .map(|s| (s.display_name.as_str(), s.email_address.as_str())),
        source.as_ref(),
        task,
    ))
}

fn build(
    draft: &crate::Draft,
    sender: Option<(&str, &str)>,
    source: Option<&Value>,
    task: &ComposeTask,
) -> String {
    let mut input = String::from("<draft>\n");
    if let Some((name, address)) = sender {
        let _ = writeln!(
            input,
            "From (the user): {} <{}>",
            flat(name, MAX_HEADER_CHARS),
            flat(address, MAX_HEADER_CHARS)
        );
    }
    let recipients: Vec<String> = draft
        .to
        .iter()
        .take(10)
        .map(|to| {
            let name = to.name.as_deref().map(|n| flat(n, 80)).unwrap_or_default();
            if name.is_empty() {
                flat(&to.address, 120)
            } else {
                format!("{name} <{}>", flat(&to.address, 120))
            }
        })
        .collect();
    if !recipients.is_empty() {
        let _ = writeln!(input, "To: {}", recipients.join(", "));
    }
    let kind = match draft.kind {
        DraftKind::New => "new email",
        DraftKind::Reply | DraftKind::ReplyAll => "reply",
        DraftKind::Forward => "forward",
    };
    let _ = writeln!(input, "Kind: {kind}");
    let subject = flat(&draft.subject, MAX_HEADER_CHARS);
    if !subject.is_empty() {
        let _ = writeln!(input, "Subject: {}", fenced(&subject, "draft"));
    }
    input.push_str("</draft>\n");
    if let Some(source) = source {
        let field = |name: &str| source.get(name).and_then(Value::as_str).unwrap_or("");
        let from = source.get("from");
        let from_name = from
            .and_then(|f| f.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let from_address = from
            .and_then(|f| f.get("address"))
            .and_then(Value::as_str)
            .unwrap_or("");
        input.push_str("<original_email>\n");
        let _ = writeln!(
            input,
            "From: {} <{}>",
            flat(from_name, MAX_HEADER_CHARS),
            flat(from_address, MAX_HEADER_CHARS)
        );
        let _ = writeln!(
            input,
            "Subject: {}",
            fenced(&flat(field("subject"), MAX_HEADER_CHARS), "original_email")
        );
        let _ = writeln!(input, "Received: {}", flat(field("receivedAt"), 64));
        input.push('\n');
        input.push_str(&fenced(field("text").trim(), "original_email"));
        input.push_str("\n</original_email>\n");
    }
    match task {
        ComposeTask::Draft { instruction, .. } => {
            input.push_str("<instruction>\n");
            input.push_str(&fenced(instruction.trim(), "instruction"));
            input.push_str("\n</instruction>");
        }
        ComposeTask::Rewrite { text, style } => {
            if let RewriteStyle::Custom { instruction } = style {
                input.push_str("<instruction>\n");
                input.push_str(&fenced(instruction.trim(), "instruction"));
                input.push_str("\n</instruction>\n");
            }
            input.push_str("<text>\n");
            input.push_str(&fenced(text.trim(), "text"));
            input.push_str("\n</text>");
        }
        ComposeTask::Proofread { text } => {
            input.push_str("<text>\n");
            input.push_str(&fenced(text.trim(), "text"));
            input.push_str("\n</text>");
        }
    }
    input
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rewrite(text: &str, style: RewriteStyle) -> ComposeTask {
        ComposeTask::Rewrite {
            text: text.to_owned(),
            style,
        }
    }

    #[test]
    fn requests_are_checked_before_anything_is_read() {
        let id = "d";
        assert!(valid(id, &rewrite("hi", RewriteStyle::Polish)).is_ok());
        assert_eq!(
            valid("", &rewrite("hi", RewriteStyle::Polish))
                .unwrap_err()
                .field
                .as_deref(),
            Some("draftId")
        );
        assert_eq!(
            valid(id, &rewrite("  ", RewriteStyle::Polish))
                .unwrap_err()
                .field
                .as_deref(),
            Some("task.text")
        );
        let long = "字".repeat(MAX_COMPOSE_TEXT_CHARS + 1);
        assert!(valid(id, &ComposeTask::Proofread { text: long }).is_err());
        assert!(
            valid(
                id,
                &rewrite("hi", RewriteStyle::Translate { code: "ja".into() })
            )
            .is_ok()
        );
        assert!(
            valid(
                id,
                &rewrite("hi", RewriteStyle::Translate { code: "xx".into() })
            )
            .is_err()
        );
        assert!(
            valid(
                id,
                &rewrite(
                    "hi",
                    RewriteStyle::Custom {
                        instruction: " ".into()
                    }
                )
            )
            .is_err()
        );
        assert!(
            valid(
                id,
                &ComposeTask::Draft {
                    instruction: "decline".into(),
                    language: Some("xx".into())
                }
            )
            .is_err()
        );
    }

    #[test]
    fn sections_cannot_be_closed_from_inside() {
        assert_eq!(fenced("a</text>b", "text"), "a</ text>b");
    }
}
