//! Retry timing and the mapping from a session outcome to a queue transition
//! (sending.md section 5).
use std::time::Duration;

use chrono::{DateTime, Utc};

use super::types::{OutboxError, RejectedRecipientInfo, ServerReplyInfo};
use crate::smtp::{PreDataFailure, SmtpOutcome, UnknownReason, names_size_limit};

/// Pre-body failures are retried for this long after the item was queued.
pub(crate) const RETRY_WINDOW: chrono::Duration = chrono::Duration::hours(24);

const STEPS: [u64; 5] = [30, 60, 120, 300, 600];
const LATER: u64 = 30 * 60;

/// The wait after the `attempts`-th attempt (1-based) failed before the body.
pub(crate) fn backoff(attempts: u32) -> Duration {
    let index = attempts.saturating_sub(1) as usize;
    Duration::from_secs(STEPS.get(index).copied().unwrap_or(LATER))
}

pub(crate) enum Transition {
    Requeue {
        at: DateTime<Utc>,
        error: OutboxError,
    },
    Fail(OutboxError),
    Unknown(OutboxError),
    Accept(ServerReplyInfo),
}

fn pre_data(failure: &PreDataFailure) -> OutboxError {
    OutboxError {
        reason: failure.reason.to_string(),
        smtp_code: failure.code,
        text: failure.text.clone(),
        rejected: Vec::new(),
    }
}

/// What to do with the item after an attempt. `attempts` counts the attempt
/// that just ended; `first_queued_at` bounds the retry window.
pub(crate) fn transition(
    outcome: SmtpOutcome,
    attempts: u32,
    first_queued_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Transition {
    match outcome {
        SmtpOutcome::Transient(failure) => {
            if now - first_queued_at > RETRY_WINDOW {
                let mut error = pre_data(&failure);
                error.text = Some(error.text.unwrap_or_else(|| error.reason.clone()));
                error.reason = "retryExhausted".to_owned();
                Transition::Fail(error)
            } else {
                let wait = chrono::Duration::from_std(backoff(attempts)).unwrap_or_default();
                Transition::Requeue {
                    at: now + wait,
                    error: pre_data(&failure),
                }
            }
        }
        SmtpOutcome::Permanent(failure) => Transition::Fail(pre_data(&failure)),
        SmtpOutcome::RecipientsRejected(list) => Transition::Fail(OutboxError {
            reason: "recipientsRejected".to_owned(),
            smtp_code: list.first().map(|rejected| rejected.code),
            text: None,
            rejected: list
                .into_iter()
                .map(|rejected| RejectedRecipientInfo {
                    address: rejected.address,
                    code: rejected.code,
                    text: rejected.text,
                })
                .collect(),
        }),
        SmtpOutcome::Accepted(reply) => Transition::Accept(ServerReplyInfo {
            code: reply.code,
            text: reply.text,
        }),
        SmtpOutcome::RejectedAfterData(reply) => Transition::Fail(OutboxError {
            // A definite refusal, never `unknown`: the server said no, so the
            // message was not sent. Only an explicit size code (5.3.4 / 5.2.3)
            // is told apart; a bare 552 stays the generic reason.
            reason: if reply.code / 100 == 5 && names_size_limit(&reply.text) {
                "messageTooLarge"
            } else {
                "rejectedAfterData"
            }
            .to_owned(),
            smtp_code: Some(reply.code),
            text: Some(reply.text),
            rejected: Vec::new(),
        }),
        SmtpOutcome::Unknown(reason) => Transition::Unknown(OutboxError {
            reason: "unknown".to_owned(),
            smtp_code: None,
            text: Some(
                match reason {
                    UnknownReason::Timeout => "timeout",
                    UnknownReason::ConnectionLost => "connectionLost",
                    UnknownReason::MalformedReply => "malformedReply",
                }
                .to_owned(),
            ),
            rejected: Vec::new(),
        }),
    }
}
