//! Draft content rules. Names, addresses and the subject end up in message
//! headers, so control characters (CR and LF above all) never pass.
use crate::{AppError, mail::MailAddress};

pub(crate) const MAX_RECIPIENTS: usize = 100;
pub(crate) const MAX_SUBJECT_BYTES: usize = 998;
pub(crate) const MAX_BODY_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_NAME_CHARS: usize = 256;

/// The editable part of a draft.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DraftContent {
    pub to: Vec<MailAddress>,
    pub cc: Vec<MailAddress>,
    pub bcc: Vec<MailAddress>,
    pub subject: String,
    pub body_text: String,
}

/// A conservative address check: one `@`, an ASCII dot-atom local part and an
/// ASCII host name. Quoted local parts, comments and internationalized
/// addresses are refused rather than guessed at.
pub(crate) fn valid_address(address: &str) -> bool {
    const ATEXT: &str = "!#$%&'*+/=?^_`{|}~-";
    if address.len() > 254 {
        return false;
    }
    let Some((local, domain)) = address.split_once('@') else {
        return false;
    };
    if local.is_empty()
        || local.len() > 64
        || local.starts_with('.')
        || local.ends_with('.')
        || local.contains("..")
        || !local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || ATEXT.contains(c))
    {
        return false;
    }
    !domain.is_empty()
        && domain.len() <= 253
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

/// Trims, drops an empty name and validates; `None` when the entry cannot be
/// used in a header.
pub(crate) fn clean_address(entry: &MailAddress) -> Option<MailAddress> {
    let address = entry.address.trim();
    let name = entry
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty());
    let name_ok = name.is_none_or(|name| {
        name.chars().count() <= MAX_NAME_CHARS && !name.chars().any(char::is_control)
    });
    (valid_address(address) && name_ok).then(|| MailAddress {
        name: name.map(str::to_owned),
        address: address.to_owned(),
    })
}

fn clean_list(list: &[MailAddress], field: &str) -> Result<Vec<MailAddress>, AppError> {
    list.iter()
        .map(|entry| clean_address(entry).ok_or_else(|| AppError::validation(field)))
        .collect()
}

/// Validates client input and returns its canonical form.
pub(crate) fn normalize(content: DraftContent) -> Result<DraftContent, AppError> {
    let to = clean_list(&content.to, "draft.to")?;
    let cc = clean_list(&content.cc, "draft.cc")?;
    let bcc = clean_list(&content.bcc, "draft.bcc")?;
    if to.len() + cc.len() + bcc.len() > MAX_RECIPIENTS {
        return Err(AppError::validation("draft.recipients"));
    }
    if content.subject.len() > MAX_SUBJECT_BYTES || content.subject.chars().any(char::is_control) {
        return Err(AppError::validation("draft.subject"));
    }
    if content.body_text.len() > MAX_BODY_BYTES || content.body_text.contains('\0') {
        return Err(AppError::validation("draft.bodyText"));
    }
    Ok(DraftContent {
        to,
        cc,
        bcc,
        subject: content.subject,
        body_text: content.body_text,
    })
}
