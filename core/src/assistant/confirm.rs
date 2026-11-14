//! Builds what the user is asked to confirm. Everything shown comes from the
//! core's current data (the draft, the queue item, the emails), looked up
//! under the run's grant; the model's arguments only say which records. This
//! blocks on storage, so it runs on a blocking thread.
use std::{collections::HashSet, path::Path};

use serde_json::Value;

use super::types::{
    ConfirmAction, ConfirmAttachment, ConfirmDraftChange, ConfirmWarning, ConfirmationDetails,
    ConfirmationRequest, OrganizeChange,
};
use crate::{AppError, DraftOrigin, ErrorCode, MailxCore, tools::Grant};

/// Characters of the body shown on a send confirmation.
pub(crate) const BODY_PREVIEW_CHARS: usize = 300;
/// Subjects listed for an organize confirmation.
const MAX_SUBJECTS: usize = 5;
const MAX_SUBJECT_CHARS: usize = 200;

fn not_found() -> AppError {
    AppError::new(ErrorCode::NotFound)
}

fn str_arg<'a>(arguments: &'a Value, field: &str) -> Option<&'a str> {
    arguments.get(field).and_then(Value::as_str)
}

fn clip(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// The draft the call names, if it is in the grant's scope.
fn scoped_draft(
    core: &MailxCore,
    grant: &Grant,
    arguments: &Value,
) -> Result<crate::Draft, AppError> {
    let id = str_arg(arguments, "draftId").ok_or_else(|| AppError::validation("draftId"))?;
    let draft = core.get_draft(id)?;
    if grant.accounts.permits(&draft.account_id) {
        Ok(draft)
    } else {
        Err(not_found())
    }
}

fn scoped_outbox(
    core: &MailxCore,
    grant: &Grant,
    arguments: &Value,
) -> Result<crate::OutboxItem, AppError> {
    let id = str_arg(arguments, "outboxId").ok_or_else(|| AppError::validation("outboxId"))?;
    let item = core.get_outbox(id)?;
    if grant.accounts.permits(&item.account_id) {
        Ok(item)
    } else {
        Err(not_found())
    }
}

/// `seen` holds the lowercased addresses of the mail the assistant read or
/// searched in this run.
pub(crate) fn build(
    core: &MailxCore,
    grant: &Grant,
    tool: &str,
    arguments: &Value,
    seen: &HashSet<String>,
) -> Result<ConfirmationRequest, AppError> {
    let (action, details) = match tool {
        "send_email" => {
            if str_arg(arguments, "action") == Some("retry") {
                let item = scoped_outbox(core, grant, arguments)?;
                (ConfirmAction::Send, outbox_details(item, true))
            } else {
                (
                    ConfirmAction::Send,
                    send_details(core, grant, arguments, seen)?,
                )
            }
        }
        "cancel_outbox" => {
            let item = scoped_outbox(core, grant, arguments)?;
            (ConfirmAction::CancelSend, outbox_details(item, false))
        }
        "set_email_flags" | "archive_emails" | "unarchive_emails" | "refresh_sync" => (
            ConfirmAction::Organize,
            organize_details(core, grant, tool, arguments)?,
        ),
        "create_draft" => (
            ConfirmAction::AiDraft,
            ConfirmationDetails::Draft {
                subject: clip(
                    str_arg(arguments, "subject").unwrap_or_default(),
                    MAX_SUBJECT_CHARS,
                ),
                origin: DraftOrigin::Ai,
                change: ConfirmDraftChange::Create,
            },
        ),
        "create_reply_draft" => {
            let id =
                str_arg(arguments, "emailId").ok_or_else(|| AppError::validation("emailId"))?;
            let mail = core.get_email(id)?;
            if !grant.accounts.permits(&mail.summary.account_id) {
                return Err(not_found());
            }
            (
                ConfirmAction::AiDraft,
                ConfirmationDetails::Draft {
                    subject: clip(&mail.summary.subject, MAX_SUBJECT_CHARS),
                    origin: DraftOrigin::Ai,
                    change: ConfirmDraftChange::Create,
                },
            )
        }
        "update_draft" | "delete_draft" | "add_draft_attachment" | "remove_draft_attachment" => {
            let draft = scoped_draft(core, grant, arguments)?;
            let change = match tool {
                "update_draft" => ConfirmDraftChange::Update {
                    fields: ["to", "cc", "bcc", "subject", "bodyHtml"]
                        .into_iter()
                        .filter(|field| arguments.get(*field).is_some_and(|v| !v.is_null()))
                        .map(|field| if field == "bodyHtml" { "body" } else { field })
                        .map(str::to_owned)
                        .collect(),
                },
                "delete_draft" => ConfirmDraftChange::Delete,
                "add_draft_attachment" => ConfirmDraftChange::AttachmentAdded {
                    file_name: clip(&added_file_name(arguments), MAX_SUBJECT_CHARS),
                },
                _ => {
                    let id = str_arg(arguments, "attachmentId")
                        .ok_or_else(|| AppError::validation("attachmentId"))?;
                    let attachment = draft
                        .attachments
                        .iter()
                        .find(|attachment| attachment.id == id)
                        .ok_or_else(not_found)?;
                    ConfirmDraftChange::AttachmentRemoved {
                        file_name: clip(&attachment.file_name, MAX_SUBJECT_CHARS),
                    }
                }
            };
            let action = match draft.origin {
                DraftOrigin::Ai => ConfirmAction::AiDraft,
                DraftOrigin::User | DraftOrigin::Mcp => ConfirmAction::UserDraft,
            };
            (
                action,
                ConfirmationDetails::Draft {
                    subject: clip(&draft.subject, MAX_SUBJECT_CHARS),
                    origin: draft.origin,
                    change,
                },
            )
        }
        _ => return Err(AppError::new(ErrorCode::Internal)),
    };
    Ok(ConfirmationRequest {
        action,
        tool: tool.to_owned(),
        details,
    })
}

/// The name the file will carry: `fileName`, else the path's last part.
fn added_file_name(arguments: &Value) -> String {
    str_arg(arguments, "fileName")
        .filter(|name| !name.trim().is_empty())
        .map(str::to_owned)
        .or_else(|| {
            str_arg(arguments, "filePath")
                .and_then(|path| Path::new(path).file_name())
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_default()
}

fn outbox_details(item: crate::OutboxItem, retry: bool) -> ConfirmationDetails {
    ConfirmationDetails::Outbox {
        retry,
        sender_address: item.sender_email_address,
        subject: clip(&item.subject, MAX_SUBJECT_CHARS),
        recipients: item.recipients,
        recipient_count: item.recipient_count,
    }
}

fn send_details(
    core: &MailxCore,
    grant: &Grant,
    arguments: &Value,
    seen: &HashSet<String>,
) -> Result<ConfirmationDetails, AppError> {
    let draft = scoped_draft(core, grant, arguments)?;
    let identity = core
        .account_identities()?
        .into_iter()
        .find(|account| account.id == draft.account_id)
        .ok_or_else(not_found)?;
    let addresses: Vec<String> = {
        let mut all: Vec<String> = draft
            .to
            .iter()
            .chain(&draft.cc)
            .chain(&draft.bcc)
            .map(|recipient| recipient.address.trim().to_lowercase())
            .collect();
        all.sort();
        all.dedup();
        all
    };
    let known: HashSet<String> = core
        .known_correspondents(&draft.account_id, &addresses)?
        .into_iter()
        .collect();
    let mut warnings = Vec::new();
    if addresses.iter().any(|address| !known.contains(address)) {
        warnings.push(ConfirmWarning::NewRecipient);
    }
    if addresses.iter().any(|address| !seen.contains(address)) {
        warnings.push(ConfirmWarning::NotInConversation);
    }
    let attachments: Vec<ConfirmAttachment> = draft
        .attachments
        .iter()
        .filter(|attachment| !attachment.inline)
        .map(|attachment| ConfirmAttachment {
            file_name: clip(&attachment.file_name, MAX_SUBJECT_CHARS),
            size: attachment.size,
        })
        .collect();
    if !attachments.is_empty() {
        warnings.push(ConfirmWarning::HasAttachments);
    }
    Ok(ConfirmationDetails::Send {
        account_name: identity.display_name,
        account_address: identity.email_address,
        to: draft.to,
        cc: draft.cc,
        bcc: draft.bcc,
        subject: clip(&draft.subject, MAX_SUBJECT_CHARS),
        body_preview: clip(draft.body_text.trim(), BODY_PREVIEW_CHARS),
        attachments,
        warnings,
    })
}

fn organize_details(
    core: &MailxCore,
    grant: &Grant,
    tool: &str,
    arguments: &Value,
) -> Result<ConfirmationDetails, AppError> {
    let change = match tool {
        "archive_emails" => OrganizeChange::Archive,
        "unarchive_emails" => OrganizeChange::Unarchive,
        "refresh_sync" => OrganizeChange::Refresh,
        _ => OrganizeChange::Flags {
            is_read: arguments.get("isRead").and_then(Value::as_bool),
            is_starred: arguments.get("isStarred").and_then(Value::as_bool),
        },
    };
    let mut count = 0u32;
    let mut subjects = Vec::new();
    if tool != "refresh_sync" {
        let ids = arguments
            .get("emailIds")
            .and_then(Value::as_array)
            .ok_or_else(|| AppError::validation("emailIds"))?;
        // The tools take at most 50 ids; anything longer fails there.
        for id in ids.iter().filter_map(Value::as_str).take(50) {
            let Ok(mail) = core.get_email(id) else {
                continue;
            };
            if !grant.accounts.permits(&mail.summary.account_id) {
                continue;
            }
            count += 1;
            if subjects.len() < MAX_SUBJECTS {
                subjects.push(clip(&mail.summary.subject, MAX_SUBJECT_CHARS));
            }
        }
    }
    Ok(ConfirmationDetails::Organize {
        change,
        count,
        subjects,
    })
}
