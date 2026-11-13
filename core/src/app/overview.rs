//! The read-only mail overview behind the Now page (now-page.md section 4).
//!
//! Everything here is computed locally from what is already stored: no body
//! is downloaded and nothing leaves the device. Drafts, the outbox, sync
//! problems and per-folder unread counts reach hosts through their own APIs
//! and are not repeated here.
use std::fmt;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::MailxCore;
use crate::{
    AppError,
    accounts::{AccountService, messages::normalized_time},
    mail::{
        EmailSummary, MailAddress,
        verification_code::{self, Detection},
    },
};

/// Newest new messages listed with their counts.
pub const OVERVIEW_LATEST: u32 = 5;
/// How far back a verification code is shown.
pub const OVERVIEW_CODE_MINUTES: i64 = 15;
/// Most verification codes listed.
pub const OVERVIEW_CODES: usize = 3;
/// Most recent messages a code is searched in; bounds the detector work.
const CODE_CANDIDATES: u32 = 50;
/// How far back attachments are listed.
pub const OVERVIEW_ATTACHMENT_DAYS: i64 = 7;
/// Most messages with attachments listed.
pub const OVERVIEW_ATTACHMENTS: u32 = 5;

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MailOverviewQuery {
    /// RFC 3339; the last time the user looked at the Now page. `None` asks
    /// for no new-mail section: `newMail` is then empty. The caller decides
    /// what to send on first use (for example the start of today).
    pub since: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MailOverview {
    pub new_mail: NewMailOverview,
    /// Starred unread messages, in the Starred view's scope.
    pub starred_unread: u32,
    /// Newest first, at most three, from the last fifteen minutes.
    pub verification_codes: Vec<OverviewCode>,
    /// Newest first, at most five inbox messages from the last seven days.
    pub recent_attachments: Vec<RecentAttachment>,
    /// RFC 3339; the core's clock when the overview was computed. The code
    /// and attachment windows end here.
    pub generated_at: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct NewMailOverview {
    pub total: u32,
    /// Only accounts with new mail, ordered by account id.
    pub accounts: Vec<AccountMailCount>,
    /// The newest five, newest first.
    pub latest: Vec<EmailSummary>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AccountMailCount {
    pub account_id: String,
    pub count: u32,
}

/// A verification code found in a recent inbox message. Debug output hides
/// the code so it never reaches a log.
#[derive(Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct OverviewCode {
    pub email_id: String,
    pub account_id: String,
    pub folder_id: String,
    pub from: MailAddress,
    pub code: String,
    /// RFC 3339.
    pub received_at: String,
}

impl fmt::Debug for OverviewCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OverviewCode")
            .field("email_id", &self.email_id)
            .field("account_id", &self.account_id)
            .field("code", &"[redacted]")
            .field("received_at", &self.received_at)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RecentAttachment {
    pub email: EmailSummary,
    /// Names of the non-inline attachments, known once the body has been
    /// downloaded; empty before that. Untrusted text from the sender.
    pub file_names: Vec<String>,
}

impl MailxCore {
    /// The Now page's local statistics. Read-only, bounded and cheap enough
    /// to run after every sync event; the time windows use the core's clock.
    pub fn mail_overview(&self, query: &MailOverviewQuery) -> Result<MailOverview, AppError> {
        let since = validate(query)?;
        overview_at(&*self.accounts()?, since.as_deref(), Utc::now())
    }
}

fn validate(query: &MailOverviewQuery) -> Result<Option<String>, AppError> {
    query
        .since
        .as_deref()
        .map(|since| normalized_time(since, "since"))
        .transpose()
}

fn timestamp(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// The overview as of `now`; `since` is already normalized.
pub(crate) fn overview_at(
    service: &AccountService,
    since: Option<&str>,
    now: DateTime<Utc>,
) -> Result<MailOverview, AppError> {
    let new_mail = match since {
        None => NewMailOverview::default(),
        Some(since) => {
            let (counts, latest) = service.overview_new_mail(since, OVERVIEW_LATEST)?;
            NewMailOverview {
                total: counts
                    .iter()
                    .fold(0u32, |sum, (_, count)| sum.saturating_add(*count)),
                accounts: counts
                    .into_iter()
                    .map(|(account_id, count)| AccountMailCount { account_id, count })
                    .collect(),
                latest,
            }
        }
    };
    let code_since = timestamp(now - Duration::minutes(OVERVIEW_CODE_MINUTES));
    let verification_codes = service
        .overview_code_sources(
            &code_since,
            CODE_CANDIDATES,
            verification_code::MAX_TEXT_BYTES,
        )?
        .into_iter()
        .filter_map(|source| {
            // Oversized bodies, ambiguous and missing codes and detector
            // errors are all skipped without a trace.
            let text = source.text?;
            match verification_code::detect(&source.subject, &text) {
                Ok(Detection::Found { candidate }) => Some(OverviewCode {
                    email_id: source.email_id,
                    account_id: source.account_id,
                    folder_id: source.folder_id,
                    from: source.from,
                    code: candidate.code,
                    received_at: source.received_at,
                }),
                _ => None,
            }
        })
        .take(OVERVIEW_CODES)
        .collect();
    let attachment_since = timestamp(now - Duration::days(OVERVIEW_ATTACHMENT_DAYS));
    let recent_attachments = service
        .overview_attachments(&attachment_since, OVERVIEW_ATTACHMENTS)?
        .into_iter()
        .map(|(email, file_names)| RecentAttachment { email, file_names })
        .collect();
    Ok(MailOverview {
        new_mail,
        starred_unread: service.overview_starred_unread()?,
        verification_codes,
        recent_attachments,
        generated_at: timestamp(now),
    })
}

#[cfg(test)]
mod tests;
