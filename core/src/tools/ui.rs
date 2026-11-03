//! Tools that act on the host UI rather than on data. Only the local MCP
//! adapter sees them; the built-in agent already lives inside the window.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::context::{Action, Audience};
use super::error::ToolError;
use super::spec::{Annotations, AudiencePolicy, ToolCall, ToolSpec, Tools};

const MAX_ID_CHARS: usize = 64;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShowEmailInput {
    /// Id of a message returned by search_emails or read_email.
    pub email_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ShowEmailOutput {
    /// The host accepted the request; the user may still dismiss the view.
    pub opened: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShowDraftInput {
    pub draft_id: String,
}

pub(super) fn register(tools: &mut Tools) {
    let show = ToolSpec::new::<ShowEmailInput, ShowEmailOutput>(
        "show_email_in_app",
        "Show an email in MailX",
        "Bring the MailX window to the front (showing it if it is hidden) and open one message for the user. \
         Changes no mail or settings, but it does move the user's focus to MailX, so call it only when the user asked to see a message. \
         The message must be inside the permitted accounts; unknown and out-of-scope ids are both reported as notFound. \
         unavailable means the MailX window cannot be controlled right now.",
        Annotations::READ_ONLY,
    )
    .requires(&[Action::MailRead])
    .visible_to(Audience::Mcp, AudiencePolicy::DEFAULT);
    tools.register(show, show_email_in_app);

    let show_draft = ToolSpec::new::<ShowDraftInput, ShowEmailOutput>(
        "show_draft_in_app",
        "Show a draft in MailX",
        "Open or focus the existing draft's editor in MailX for the user to review, edit and send themselves. \
         This never sends, deletes, or changes a draft. It moves focus to MailX, so only call it when the user \
         asked to see or review the draft. Unknown and out-of-scope draft ids are both notFound.",
        Annotations::READ_ONLY,
    )
    .requires(&[Action::MailRead])
    .visible_to(Audience::Mcp, AudiencePolicy::DEFAULT);
    tools.register(show_draft, show_draft_in_app);
}

fn show_draft_in_app(
    call: &ToolCall<'_>,
    input: ShowDraftInput,
) -> Result<ShowEmailOutput, ToolError> {
    if input.draft_id.is_empty() || input.draft_id.chars().count() > MAX_ID_CHARS {
        return Err(ToolError::invalid("draftId", "invalid"));
    }
    let draft = call.core.get_draft(&input.draft_id)?;
    call.require_account(&draft.account_id)?;
    call.note_resource(&draft.id);
    call.open_draft_in_app(&draft.account_id, &draft.id)?;
    Ok(ShowEmailOutput { opened: true })
}

fn show_email_in_app(
    call: &ToolCall<'_>,
    input: ShowEmailInput,
) -> Result<ShowEmailOutput, ToolError> {
    if input.email_id.is_empty() || input.email_id.chars().count() > MAX_ID_CHARS {
        return Err(ToolError::invalid("emailId", "invalid"));
    }
    // Scope first: the hook must never run for a message outside the grant.
    let detail = call.core.get_email(&input.email_id)?;
    call.require_account(&detail.summary.account_id)?;
    call.note_resource(&detail.summary.id);
    call.open_in_app(&detail.summary.account_id, &detail.summary.id)?;
    Ok(ShowEmailOutput { opened: true })
}
