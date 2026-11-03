//! Automatic sending and bounded outbox access through the shared tool layer.
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::context::{AccountScope, Action, Audience};
use super::error::{ToolError, ToolErrorCode};
use super::spec::{Annotations, AudiencePolicy, ToolCall, ToolSpec, Tools};
use crate::{OutboxItem, OutboxResolution, OutboxState, QueueSendRequest};

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SendAction {
    Send,
    Retry,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SendEmailInput {
    pub action: SendAction,
    /// Required only for send.
    #[serde(default, deserialize_with = "non_null")]
    pub draft_id: Option<String>,
    /// Required only for send; use the revision from read_draft.
    #[serde(default, deserialize_with = "non_null")]
    pub expected_revision: Option<u32>,
    /// Required UUID for send. Reuse it for the same request after a timeout.
    #[serde(default, deserialize_with = "non_null")]
    pub client_mutation_id: Option<String>,
    /// Required only for retry; the item must still be queued.
    #[serde(default, deserialize_with = "non_null")]
    pub outbox_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OutboxIdInput {
    pub outbox_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListOutboxInput {
    pub account_id: Option<String>,
    /// Omit for queued, sending, failed and unresolved unknown items.
    pub states: Option<Vec<OutboxState>>,
    /// Defaults to 20, at most 100.
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OutboxFailure {
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smtp_code: Option<u16>,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct OutboxInfo {
    pub id: String,
    pub account_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft_id: Option<String>,
    pub subject: String,
    /// Up to three envelope recipients; recipientCount includes all of them.
    pub recipients: Vec<String>,
    pub recipient_count: u32,
    /// accepted is not delivery confirmation; resolution records user overrides.
    pub state: OutboxState,
    pub not_before: String,
    pub attempts: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_attempt_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepted_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<OutboxFailure>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolution: Option<OutboxResolution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListOutboxOutput {
    pub items: Vec<OutboxInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

// Queue errors originate in fixed internal codes. Keep the tool boundary
// conservative if a migrated or damaged row contains arbitrary text here.
fn safe_reason(reason: &str) -> &str {
    match reason {
        "connect"
        | "proxy"
        | "timeout"
        | "network"
        | "protocol"
        | "server"
        | "tlsVerification"
        | "tlsUnavailable"
        | "authUnavailable"
        | "authRejected"
        | "messageTooLarge"
        | "credentialUntrusted"
        | "credentialMissing"
        | "secretStore"
        | "account"
        | "dataMarkFailed"
        | "invalidMessage"
        | "retryExhausted"
        | "recipientsRejected"
        | "rejectedAfterData"
        | "unknown"
        | "interrupted"
        | "accountChanged"
        | "snapshotMissing" => reason,
        _ => "other",
    }
}

impl From<OutboxItem> for OutboxInfo {
    fn from(item: OutboxItem) -> Self {
        Self {
            id: item.id,
            account_id: item.account_id,
            draft_id: item.draft_id,
            subject: item.subject.chars().take(998).collect(),
            recipients: item
                .recipients
                .into_iter()
                .take(3)
                .map(|address| address.chars().take(320).collect())
                .collect(),
            recipient_count: item.recipient_count,
            state: item.state,
            not_before: item.not_before,
            attempts: item.attempts,
            next_attempt_at: item.next_attempt_at,
            accepted_at: item.accepted_at,
            error: item.error.map(|error| OutboxFailure {
                reason: safe_reason(&error.reason).to_owned(),
                smtp_code: error.smtp_code,
            }),
            resolution: item.resolution,
            superseded_by: item.superseded_by,
            created_at: item.created_at,
            updated_at: item.updated_at,
        }
    }
}

pub(super) fn register(tools: &mut Tools) {
    let send = ToolSpec::new::<SendEmailInput, OutboxInfo>(
        "send_email", "Send email",
        "Send a saved draft automatically, or retry a queued item without bypassing its undo window. \
         Reuse clientMutationId when repeating the same send request. Failed or unknown items cannot be retried.",
        Annotations {
            writes_server_data: true,
            destructive: true,
            open_world: true,
            ..Annotations::WRITES_LOCAL
        },
    )
    .requires(&[Action::MailSend])
    .visible_to(Audience::BuiltinAgent, AudiencePolicy::DEFAULT)
    .visible_to(Audience::Mcp, AudiencePolicy::DEFAULT);
    tools.register(send, send_email);

    let list = ToolSpec::new::<ListOutboxInput, ListOutboxOutput>(
        "list_outbox", "List outbox",
        "List sending tasks newest first. Omit states for tasks still pending or needing attention.",
        Annotations::READ_ONLY,
    )
    .requires(&[Action::OutboxRead])
    .visible_to(Audience::BuiltinAgent, AudiencePolicy::DEFAULT)
    .visible_to(Audience::Mcp, AudiencePolicy::DEFAULT);
    tools.register(list, list_outbox);

    let get = ToolSpec::new::<OutboxIdInput, OutboxInfo>(
        "get_outbox", "Read sending status",
        "Read a sending task. accepted is not delivery confirmation; unknown may already have been sent and must not be resent automatically.",
        Annotations::READ_ONLY,
    )
    .requires(&[Action::OutboxRead])
    .visible_to(Audience::BuiltinAgent, AudiencePolicy::DEFAULT)
    .visible_to(Audience::Mcp, AudiencePolicy::DEFAULT);
    tools.register(
        get,
        |call, input: OutboxIdInput| -> Result<OutboxInfo, ToolError> {
            Ok(outbox(call, &input.outbox_id)?.into())
        },
    );

    let cancel = ToolSpec::new::<OutboxIdInput, OutboxInfo>(
        "cancel_outbox", "Cancel queued email",
        "Cancel a queued sending task and keep its draft. An item already sending cannot be canceled.",
        Annotations::WRITES_LOCAL,
    )
    .requires(&[Action::OutboxWrite])
    .visible_to(Audience::BuiltinAgent, AudiencePolicy::DEFAULT)
    .visible_to(Audience::Mcp, AudiencePolicy::DEFAULT);
    tools.register(
        cancel,
        |call, input: OutboxIdInput| -> Result<OutboxInfo, ToolError> {
            outbox(call, &input.outbox_id)?;
            Ok(call.core.cancel_outbox(&input.outbox_id)?.into())
        },
    );
}

fn non_null<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

fn uuid(field: &str, value: &str) -> Result<(), ToolError> {
    if value.len() > 64 || uuid::Uuid::parse_str(value).is_err() {
        return Err(ToolError::invalid(field, "expected a UUID"));
    }
    Ok(())
}

fn scope<'a>(call: &'a ToolCall<'_>) -> Option<&'a [String]> {
    match &call.ctx.grant.accounts {
        AccountScope::All => None,
        AccountScope::Only(ids) => Some(ids),
    }
}

fn outbox(call: &ToolCall<'_>, id: &str) -> Result<OutboxItem, ToolError> {
    uuid("outboxId", id)?;
    call.note_resource(id);
    let item = call.core.get_outbox(id)?;
    call.require_account(&item.account_id)?;
    call.note_resource(&item.account_id);
    Ok(item)
}

fn send_email(call: &ToolCall<'_>, input: SendEmailInput) -> Result<OutboxInfo, ToolError> {
    match input.action {
        SendAction::Send => {
            if input.outbox_id.is_some() {
                return Err(ToolError::invalid("outboxId", "not allowed for send"));
            }
            let draft_id = input
                .draft_id
                .ok_or_else(|| ToolError::invalid("draftId", "required for send"))?;
            let expected_revision = input
                .expected_revision
                .ok_or_else(|| ToolError::invalid("expectedRevision", "required for send"))?;
            let client_mutation_id = input
                .client_mutation_id
                .ok_or_else(|| ToolError::invalid("clientMutationId", "required for send"))?;
            uuid("draftId", &draft_id)?;
            uuid("clientMutationId", &client_mutation_id)?;
            call.note_resource(&draft_id);
            let request = QueueSendRequest {
                draft_id,
                expected_revision,
                client_mutation_id,
            };
            // A successful send deletes its draft. Look up the scoped mutation
            // first so a lost response cannot cause a second delivery attempt.
            if let Some(item) = call.core.replay_send_scoped(&request, scope(call))? {
                call.note_resource(&item.account_id);
                call.note_resource(&item.id);
                return Ok(item.into());
            }
            let draft = call.core.get_draft(&request.draft_id)?;
            call.require_account(&draft.account_id)?;
            call.note_resource(&draft.account_id);
            if call.draft_open(&request.draft_id) {
                return Err(ToolError::new(ToolErrorCode::DraftOpen));
            }
            let item = call.core.queue_send_scoped(
                call.ctx.principal.caller(),
                request,
                call.undo_send_seconds()?,
                scope(call),
            )?;
            call.note_resource(&item.id);
            Ok(item.into())
        }
        SendAction::Retry => {
            if input.draft_id.is_some()
                || input.expected_revision.is_some()
                || input.client_mutation_id.is_some()
            {
                return Err(ToolError::invalid(
                    "input",
                    "retry accepts only action and outboxId",
                ));
            }
            let id = input
                .outbox_id
                .ok_or_else(|| ToolError::invalid("outboxId", "required for retry"))?;
            outbox(call, &id)?;
            Ok(call.core.retry_outbox_now(&id)?.into())
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Cursor {
    version: u8,
    created_at: String,
    id: String,
    filter: String,
}

fn list_outbox(call: &ToolCall<'_>, input: ListOutboxInput) -> Result<ListOutboxOutput, ToolError> {
    let limit = input.limit.unwrap_or(call.policy.default_page_size);
    if limit == 0 || limit > call.policy.max_page_size {
        return Err(ToolError::invalid("limit", "out of range"));
    }
    let mut accounts = if let Some(id) = input.account_id {
        uuid("accountId", &id)?;
        call.require_account(&id)?;
        // A nonexistent requested account is indistinguishable from one outside scope.
        call.core.require_account_exists(&id)?;
        call.note_resource(&id);
        Some(vec![id])
    } else {
        scope(call).map(<[String]>::to_vec)
    };
    if let Some(accounts) = &mut accounts {
        accounts.sort();
        accounts.dedup();
    }
    let mut states = input.states;
    if let Some(states) = &mut states {
        if states.is_empty() || states.len() > 6 {
            return Err(ToolError::invalid("states", "expected 1 to 6 states"));
        }
        states.sort_by_key(|state| *state as u8);
        states.dedup();
    }
    let query = serde_json::to_vec(&(&accounts, &states))
        .map_err(|_| ToolError::new(ToolErrorCode::Internal))?;
    let filter = format!("{:x}", Sha256::digest(query));
    let before = input
        .cursor
        .map(|encoded| {
            let invalid = || ToolError::invalid("cursor", "invalid or different filters");
            if encoded.len() > 1024 {
                return Err(invalid());
            }
            let bytes = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| invalid())?;
            let cursor: Cursor = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
            if cursor.version != 1
                || cursor.filter != filter
                || chrono::DateTime::parse_from_rfc3339(&cursor.created_at).is_err()
                || uuid::Uuid::parse_str(&cursor.id).is_err()
            {
                return Err(invalid());
            }
            Ok(cursor)
        })
        .transpose()?;
    let mut items = call.core.list_outbox_page(
        accounts.as_deref(),
        states.as_deref(),
        before
            .as_ref()
            .map(|cursor| (cursor.created_at.as_str(), cursor.id.as_str())),
        limit + 1,
    )?;
    let has_more = items.len() > limit as usize;
    items.truncate(limit as usize);
    let next_cursor = if has_more {
        let last = items
            .last()
            .ok_or_else(|| ToolError::new(ToolErrorCode::Internal))?;
        let cursor = Cursor {
            version: 1,
            created_at: last.created_at.clone(),
            id: last.id.clone(),
            filter,
        };
        Some(URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&cursor).map_err(|_| ToolError::new(ToolErrorCode::Internal))?,
        ))
    } else {
        None
    };
    for item in &items {
        call.note_resource(&item.account_id);
        call.note_resource(&item.id);
    }
    Ok(ListOutboxOutput {
        items: items.into_iter().map(Into::into).collect(),
        next_cursor,
    })
}

#[cfg(test)]
#[path = "sending_tests.rs"]
mod tests;
