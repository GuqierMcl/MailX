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
    /// Id of a message returned by mail_search or mail_read.
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
    let show = ToolSpec::new::<ShowInAppInput, ShowEmailOutput>(
        "show_in_app",
        "Show a message or draft in MailX",
        "Bring MailX to the front and open one message (emailId) or one draft's editor (draftId) for the \
         user. It changes nothing and never sends, but it moves the user's focus, so call it only when \
         they asked to see it. Unknown and out-of-scope ids are both notFound. Details: help.",
        Annotations::READ_ONLY,
    )
    .requires(&[Action::MailRead])
    .visible_to(Audience::Mcp, AudiencePolicy::DEFAULT);
    tools.register(show, show_in_app);
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShowInAppInput {
    /// A message id from mail_search or mail_read.
    pub email_id: Option<String>,
    /// A draft id from draft_list or draft_create.
    pub draft_id: Option<String>,
}

fn show_in_app(call: &ToolCall<'_>, input: ShowInAppInput) -> Result<ShowEmailOutput, ToolError> {
    match (input.email_id, input.draft_id) {
        (Some(email_id), None) => show_email_in_app(call, ShowEmailInput { email_id }),
        (None, Some(draft_id)) => show_draft_in_app(call, ShowDraftInput { draft_id }),
        _ => Err(ToolError::invalid(
            "emailId",
            "give exactly one of emailId and draftId",
        )),
    }
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
