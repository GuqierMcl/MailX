//! One polling round for an on-premises Exchange (EWS) account.
//!
//! EWS has no UIDs, no IDLE and no partial fetch. ItemIds are translated into
//! stable local uids by `storage/ews.rs`, and whole MIME messages are fetched
//! and rendered during the round so the on-demand body and preview readers
//! never open an IMAP session for this account. The worker reuses the manager's
//! cadence, generation guards and outcome handling; only the protocol differs.
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{Duration, Utc};

use super::manager::{Outcome, Shared, SyncError, WorkerToken, from_sync_error, with_store};
use crate::accounts::sync_plan::PlanError;
use crate::error::{AppError, ErrorCode};
use crate::ews::http::HttpError;
use crate::ews::soap::{self, Item, Party};
use crate::ews::{EwsClient, EwsError};
use crate::imap::{FolderStatus, ImapError};
use crate::mail::{FolderRole, MailAddress};
use crate::storage::Flag;
use crate::storage::messages::SyncTarget;
use crate::sync::folders::{self, FolderRecord};
use crate::sync::headers::MessageRecord;

/// EWS answers up to this many messages per folder per round. Folders with
/// more mail fill in from the newest end on later rounds.
const ITEMS_PER_FOLDER: usize = 500;
/// Only headers inside the initial sync window are worth caching.
const WINDOW_DAYS: i64 = 30;
/// EWS ItemIds are opaque; the local identity uses a fixed generation.
const UID_VALIDITY: u32 = 1;

pub(super) async fn sync_once(shared: &Arc<Shared>, token: &WorkerToken) -> Outcome {
    match round(shared, token).await {
        Ok(()) => Outcome::Synced,
        Err(error) => from_sync_error(error),
    }
}

async fn round(shared: &Arc<Shared>, token: &WorkerToken) -> Result<(), SyncError> {
    let id = token.account_id.clone();
    let planned = with_store(shared, token, move |service| Ok(service.ews_plan(&id))).await?;
    let plan = match planned {
        Ok(Some(plan)) => plan,
        // The account changed away from EWS while its worker was running.
        Ok(None) => return Err(SyncError::Stale),
        Err(PlanError::Account(error)) => return Err(SyncError::Storage(error)),
        Err(PlanError::CredentialUntrusted | PlanError::CredentialMissing) => {
            return Err(SyncError::Imap(ImapError::Authentication(
                "the saved EWS credential cannot be used".into(),
            )));
        }
    };
    if plan.mailbox_source_id != token.source {
        return Err(SyncError::Stale);
    }
    let mut client = EwsClient::connect(&plan.route, &plan.endpoint, &plan.user, &plan.password)
        .await
        .map_err(ews_failure)?;

    let folders = client.folders().await.map_err(ews_failure)?;
    let names: Vec<&str> = DISTINGUISHED_ROLES.iter().map(|(name, _)| *name).collect();
    let ids = client.distinguished_folder_ids(&names).await;
    let records = folder_records(&folders, &folder_roles_by_id(&ids));

    let account_id = token.account_id.clone();
    let source = token.source.clone();
    let targets = with_store(shared, token, move |service| {
        let listed = records;
        if !service.store_folders(&account_id, &source, &listed)? {
            return Err(AppError::new(ErrorCode::Conflict));
        }
        service.sync_folders(&account_id, &source)
    })
    .await?;
    shared.reconcile_targets(token, &targets, true);

    for (folder_id, remote_name) in &targets {
        sync_folder(shared, token, &mut client, folder_id, remote_name).await?;
    }
    write_back(shared, token, &mut client).await?;
    Ok(())
}

async fn sync_folder(
    shared: &Arc<Shared>,
    token: &WorkerToken,
    client: &mut EwsClient,
    folder_id: &str,
    remote_name: &str,
) -> Result<(), SyncError> {
    let items = client
        .items(remote_name, ITEMS_PER_FOLDER)
        .await
        .map_err(ews_failure)?;
    let account_id = token.account_id.clone();
    let source = token.source.clone();
    let folder = folder_id.to_owned();
    let remote = remote_name.to_owned();
    let keys: Vec<(String, Option<String>)> = items
        .iter()
        .map(|item| (item.id.clone(), item.change_key.clone()))
        .collect();
    let now = Utc::now();
    let window_start = (now.date_naive() - Duration::days(WINDOW_DAYS)).to_string();
    let pending = with_store(
        shared,
        token,
        move |service| -> Result<Vec<(String, String, Option<String>)>, AppError> {
            let assigned = service.ews_assign(&account_id, &remote, &keys)?;
            let target = SyncTarget {
                account_id: account_id.clone(),
                mailbox_source_id: source.clone(),
                folder_id: folder.clone(),
                uid_validity: UID_VALIDITY,
            };
            let cursor = service
                .ensure_generation(&target, &window_start)?
                .ok_or_else(|| AppError::new(ErrorCode::Conflict))?;
            let by_uid: std::collections::HashMap<u32, &Item> = items
                .iter()
                .zip(assigned.iter())
                .map(|(item, mapped)| (mapped.uid, item))
                .collect();
            let records: Vec<MessageRecord> = assigned
                .iter()
                .filter_map(|mapped| {
                    by_uid
                        .get(&mapped.uid)
                        .map(|item| message_record(mapped.uid, item))
                })
                .collect();
            let flags: Vec<(u32, Vec<String>)> = records
                .iter()
                .map(|record| (record.uid, record.flags.clone()))
                .collect();
            let highest_uid = assigned
                .iter()
                .map(|mapped| mapped.uid)
                .max()
                .unwrap_or(0)
                .max(cursor.highest_uid);
            service.begin_backfill(&target, false)?;
            service.store_message_batch(&target, &records, highest_uid)?;
            service.apply_flags(&target, &flags)?;
            service.finish_backfill(&target, false)?;
            // Only messages whose body is still missing need a GetItem round.
            let mut missing = Vec::new();
            for mapped in &assigned {
                if let Some(message_id) =
                    service.ews_message_id(&account_id, &folder, UID_VALIDITY, mapped.uid)?
                    && !service.ews_body_ready(&message_id)?
                {
                    missing.push((
                        message_id,
                        mapped.item_id.clone(),
                        mapped.change_key.clone(),
                    ));
                }
            }
            Ok(missing)
        },
    )
    .await?;

    for (message_id, item_id, change_key) in pending {
        let raw = client
            .mime(&item_id, change_key.as_deref())
            .await
            .map_err(ews_failure)?;
        let account_id = token.account_id.clone();
        with_store(shared, token, move |service| {
            service.ews_cache_body(&message_id, &raw)?;
            let _ = &account_id;
            Ok(())
        })
        .await?;
    }
    Ok(())
}

/// Applies local read/starred changes through UpdateItem. The IMAP flag
/// writer is not started for EWS accounts, so this is the only path.
async fn write_back(
    shared: &Arc<Shared>,
    token: &WorkerToken,
    client: &mut EwsClient,
) -> Result<(), SyncError> {
    let account_id = token.account_id.clone();
    let changes = with_store(shared, token, move |service| {
        service.pending_flag_changes(&account_id, 64)
    })
    .await?;
    for change in changes {
        let account_id = token.account_id.clone();
        let found = with_store(shared, token, move |service| {
            service.ews_item(&account_id, &change.remote_name, change.uid)
        })
        .await?;
        let Some((item_id, change_key)) = found else {
            continue;
        };
        let (read, flagged) = match change.flag {
            Flag::Seen => (Some(change.wanted), None),
            Flag::Flagged => (None, Some(change.wanted)),
        };
        let result = client
            .set_flags(&item_id, change_key.as_deref(), read, flagged)
            .await;
        let message_id = change.message_id.clone();
        let flag = change.flag;
        let sent = result.is_ok();
        with_store(shared, token, move |service| {
            if sent {
                service.finish_flag_change(&message_id, flag, true)?;
            } else {
                service.retry_flag_change(&message_id, flag)?;
            }
            Ok(())
        })
        .await?;
    }
    Ok(())
}

fn message_record(uid: u32, item: &Item) -> MessageRecord {
    let mut flags = Vec::new();
    if item.is_read {
        flags.push("\\Seen".to_owned());
    }
    if item.is_flagged {
        flags.push("\\Flagged".to_owned());
    }
    MessageRecord {
        uid,
        message_id_header: item.message_id.clone(),
        original_message_id: None,
        sent_identity_policy: "default:v2",
        in_reply_to: item.in_reply_to.clone(),
        references_header: item.references.clone(),
        subject: item.subject.clone(),
        from_name: item.from.as_ref().and_then(|party| party.name.clone()),
        from_address: item
            .from
            .as_ref()
            .map(|party| party.address.clone())
            .unwrap_or_default(),
        to_addresses: parties(&item.to),
        cc_addresses: parties(&item.cc),
        reply_to_addresses: parties(&item.reply_to),
        received_at: utc(item.received_at.as_deref()),
        sent_at: item.sent_at.as_deref().map(|value| utc(Some(value))),
        size: item.size,
        is_read: item.is_read,
        is_flagged: item.is_flagged,
        flags,
        has_attachment: item.has_attachments,
    }
}

fn parties(values: &[Party]) -> Vec<MailAddress> {
    values
        .iter()
        .map(|party| MailAddress {
            name: party.name.clone(),
            address: party.address.clone(),
        })
        .collect()
}

fn utc(value: Option<&str>) -> String {
    value
        .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
        .map(|date| date.with_timezone(&Utc))
        .unwrap_or_else(Utc::now)
        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
        .to_string()
}

/// Well-known EWS distinguished folders, in the exact order their ids are
/// requested. `archive` has no distinguished id — an in-place archive is a
/// separate mailbox — so Archive stays name-based.
const DISTINGUISHED_ROLES: [(&str, FolderRole); 5] = [
    ("inbox", FolderRole::Inbox),
    ("drafts", FolderRole::Drafts),
    ("sentitems", FolderRole::Sent),
    ("deleteditems", FolderRole::Trash),
    ("junkemail", FolderRole::Junk),
];

const ROLE_CANDIDATES: [FolderRole; 6] = [
    FolderRole::Inbox,
    FolderRole::Sent,
    FolderRole::Drafts,
    FolderRole::Trash,
    FolderRole::Junk,
    FolderRole::Archive,
];

/// Maps the ids `GetFolder` reported onto roles. A short or empty answer means
/// the deployment did not support the operation; the caller guesses by name.
fn folder_roles_by_id(ids: &[Option<String>]) -> HashMap<String, FolderRole> {
    let mut known = HashMap::new();
    if ids.len() != DISTINGUISHED_ROLES.len() {
        return known;
    }
    for (id, (_, role)) in ids.iter().zip(DISTINGUISHED_ROLES) {
        if let Some(id) = id {
            known.entry(id.clone()).or_insert(role);
        }
    }
    known
}

/// Turns the flat list a deep `FindFolder` returns into the records the local
/// folder cache expects.
///
/// EWS has no delimiter and no SPECIAL-USE attributes: hierarchy comes from
/// `ParentFolderId`, and roles come from `GetFolder`'s distinguished ids (or
/// well-known names when that operation is unavailable). Keeping the two paths
/// consistent matters: `replace_folders` writes `parent_id` from
/// `parent_remote_name`, and the sidebar builds its tree from it.
///
/// Some mailboxes carry empty look-alike folders — "已发送" beside
/// "已发送邮件", "Archive" beside "存档" — left by a locale switch or an old
/// client, plus Outlook's own empty bookkeeping folders (Outbox, Quick Step
/// Settings, Working Set, ...). MailX already has local Archive/Outbox/Drafts
/// views, so showing those server shells makes the sidebar look like the
/// mailbox has two of everything. The folder that actually holds mail keeps
/// its role; empty shells are dropped from the local listing.
fn folder_records(
    folders: &[soap::Folder],
    known: &HashMap<String, FolderRole>,
) -> Vec<FolderRecord> {
    let mail: Vec<&soap::Folder> = folders
        .iter()
        .filter(|folder| is_mail_folder(folder))
        .collect();
    let ids: HashSet<&str> = mail.iter().map(|folder| folder.id.as_str()).collect();
    let names: HashMap<&str, &str> = mail
        .iter()
        .map(|folder| (folder.id.as_str(), folder.display_name.as_str()))
        .collect();
    // `replace_folders` looks the parent up by remote name and indexes into
    // its id map, so a parent outside this listing must become `None`.
    let parents: Vec<Option<&str>> = mail
        .iter()
        .map(|folder| {
            folder
                .parent_id
                .as_deref()
                .filter(|parent| ids.contains(parent))
        })
        .collect();
    // Name guessing only applies at the top level or directly under Inbox
    // (the IMAP `INBOX.Sent` shape). A nested user folder called "Trash" is
    // not a system folder.
    let near_top: Vec<bool> = parents
        .iter()
        .map(|parent| {
            parent.is_none_or(|parent| names.get(parent).is_some_and(|name| is_inbox_name(name)))
        })
        .collect();
    let claimed: Vec<Option<FolderRole>> = mail
        .iter()
        .zip(&near_top)
        .map(|(folder, near)| {
            (*near)
                .then(|| claimed_role(&folder.display_name))
                .flatten()
        })
        .collect();

    let mut roles = vec![FolderRole::Other; mail.len()];
    let mut taken: HashSet<FolderRole> = HashSet::new();
    // The server's own distinguished ids are authoritative. Without them a
    // name-based guess can pick the empty alias, leaving the top-level Sent
    // view empty while the real folder sits further down as "Other".
    for (index, folder) in mail.iter().enumerate() {
        if let Some(&role) = known.get(&folder.id)
            && taken.insert(role)
        {
            roles[index] = role;
        }
    }
    // Roles the server did not identify (notably Archive, which has no
    // distinguished id) fall back to names. Whichever candidate actually
    // holds mail wins; ties keep the server's order.
    for role in ROLE_CANDIDATES {
        if taken.contains(&role) {
            continue;
        }
        let mut best: Option<usize> = None;
        for index in 0..mail.len() {
            if claimed[index] != Some(role) || roles[index] != FolderRole::Other {
                continue;
            }
            let better = best.is_none_or(|current| {
                (mail[index].total_count, mail[index].unread_count)
                    > (mail[current].total_count, mail[current].unread_count)
            });
            if better {
                best = Some(index);
            }
        }
        if let Some(index) = best {
            taken.insert(role);
            roles[index] = role;
        }
    }

    let referenced: HashSet<&str> = parents.iter().flatten().copied().collect();
    // MailX already has its own Archive, Outbox and Drafts views. An empty
    // top-level server folder that duplicates one of them — or that Outlook
    // created purely for client bookkeeping — only makes the sidebar look
    // like the mailbox has two of everything. Drop these shells from the
    // local listing; their mail stays on the server and the folder comes
    // back as soon as it holds anything. Never drop a folder with children
    // (its subtree would be flattened) or one another folder points at.
    let hidden: Vec<bool> = (0..mail.len())
        .map(|index| {
            let folder = mail[index];
            let shell = (roles[index] == FolderRole::Other && claimed[index].is_some())
                || roles[index] == FolderRole::Archive
                || is_system_shell_name(&folder.display_name);
            shell
                && parents[index].is_none()
                && !referenced.contains(folder.id.as_str())
                && folder.total_count == 0
                && folder.unread_count == 0
                && folder.child_folder_count.unwrap_or(0) == 0
        })
        .collect();
    let kept: HashSet<&str> = mail
        .iter()
        .zip(&hidden)
        .filter(|(_, hidden)| !**hidden)
        .map(|(folder, _)| folder.id.as_str())
        .collect();

    mail.iter()
        .enumerate()
        .filter(|(index, _)| !hidden[*index])
        .map(|(index, folder)| FolderRecord {
            remote_name: folder.id.clone(),
            parent_remote_name: parents[index]
                .filter(|parent| kept.contains(parent))
                .map(ToOwned::to_owned),
            name: folder.display_name.clone(),
            delimiter: None,
            role: roles[index],
            selectable: true,
            attributes: Vec::new(),
            status: Some(FolderStatus {
                messages: folder.total_count,
                unseen: folder.unread_count,
                uid_next: None,
                uid_validity: Some(UID_VALIDITY),
            }),
        })
        .collect()
}

/// The system role a folder's name claims, if any. Compared exactly so a user
/// folder such as "收件箱备份" or "Sent 2026" stays Other.
fn claimed_role(name: &str) -> Option<FolderRole> {
    if is_inbox_name(name) {
        return Some(FolderRole::Inbox);
    }
    let name = name.trim().to_lowercase();
    for (role, candidates) in folders::WELL_KNOWN_NAMES {
        if candidates.iter().any(|candidate| name == *candidate) {
            return Some(role);
        }
    }
    None
}

/// Empty top-level folders Exchange/Outlook keeps for its own bookkeeping
/// rather than for the user's mail. Compared exactly after trimming and
/// lower-casing, so a real folder such as "Files 2026" or "存档 2026" is
/// never treated as a shell.
///
/// Archive names are repeated here because a folder that *did* win the
/// Archive role is not caught by the look-alike rule; Outbox has no
/// `FolderRole` at all, since MailX's outbox is a local view.
const SYSTEM_SHELL_NAMES: &[&str] = &[
    "outbox",
    "发件箱",
    "寄件匣",
    "待发送",
    "待发送邮件",
    "archive",
    "archives",
    "归档",
    "存档",
    "封存",
    "quick step settings",
    "快速步骤设置",
    "快速步驟設定",
    "conversation action settings",
    "会话操作设置",
    "交談動作設定",
    "working set",
    "工作集",
    "files",
    "文件",
    "檔案",
    "sync issues",
    "同步问题",
    "同步問題",
    "yammer root",
    "yammer 根目录",
    "yammer 根目錄",
    "rss feeds",
    "rss 订阅",
];

fn is_system_shell_name(name: &str) -> bool {
    let name = name.trim().to_lowercase();
    SYSTEM_SHELL_NAMES
        .iter()
        .any(|candidate| name == *candidate)
}

/// A localised Exchange server names its Inbox differently from IMAP's
/// `INBOX`; compare exactly so a user folder such as "收件箱备份" stays Other.
fn is_inbox_name(name: &str) -> bool {
    folders::is_inbox(name.trim()) || matches!(name.trim(), "收件箱" | "收件匣")
}

/// Mail folders only. An explicit calendar / contact / task / journal class is
/// excluded; a missing or custom class is kept rather than hiding a folder.
fn is_mail_folder(folder: &soap::Folder) -> bool {
    if !folder.mail_folder {
        return false;
    }
    let class = folder.folder_class.as_deref().unwrap_or("").trim();
    ![
        "IPF.Appointment",
        "IPF.Contact",
        "IPF.Task",
        "IPF.StickyNote",
        "IPF.Journal",
    ]
    .iter()
    .any(|prefix| class.starts_with(prefix))
}

fn ews_failure(error: EwsError) -> SyncError {
    match error {
        EwsError::Http(HttpError::Rejected) => SyncError::Imap(ImapError::Authentication(
            "Exchange refused the sign-in".into(),
        )),
        EwsError::Http(HttpError::NoNtlm) => SyncError::Imap(ImapError::ProviderPolicy),
        _ => SyncError::Imap(ImapError::Protocol(format!(
            "EWS request failed: {error:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A deployment that does not answer `GetFolder` relies on names alone.
    fn no_distinguished(folders: &[soap::Folder]) -> Vec<FolderRecord> {
        folder_records(folders, &HashMap::new())
    }

    fn ews_folder(
        id: &str,
        parent: Option<&str>,
        name: &str,
        class: &str,
        mail: bool,
        total: u32,
        unread: u32,
    ) -> soap::Folder {
        soap::Folder {
            id: id.into(),
            parent_id: parent.map(Into::into),
            display_name: name.into(),
            total_count: total,
            unread_count: unread,
            child_folder_count: None,
            folder_class: (!class.is_empty()).then(|| class.into()),
            mail_folder: mail,
        }
    }

    #[test]
    fn well_known_folder_names_get_roles() {
        let records = no_distinguished(&[
            ews_folder("in", None, "收件箱", "IPF.Note", true, 10, 2),
            ews_folder("sent", None, "已发送邮件", "IPF.Note", true, 0, 0),
            ews_folder("draft", None, "草稿", "IPF.Note", true, 0, 0),
            ews_folder("trash", None, "Deleted Items", "IPF.Note", true, 0, 0),
            ews_folder("junk", None, "Junk Email", "IPF.Note", true, 0, 0),
            // Non-empty, so this test stays about role assignment rather than
            // the empty-shell filtering verified below.
            ews_folder("archive", None, "Archive", "IPF.Note", true, 1, 0),
            ews_folder("projects", None, "Projects", "IPF.Note", true, 0, 0),
        ]);
        let roles: Vec<FolderRole> = records.iter().map(|record| record.role).collect();
        assert_eq!(
            roles,
            [
                FolderRole::Inbox,
                FolderRole::Sent,
                FolderRole::Drafts,
                FolderRole::Trash,
                FolderRole::Junk,
                FolderRole::Archive,
                FolderRole::Other,
            ]
        );
    }

    #[test]
    fn distinguished_ids_pick_the_real_system_folder() {
        let known = HashMap::from([("real".to_owned(), FolderRole::Sent)]);
        let records = folder_records(
            &[
                ews_folder("alias", None, "已发送", "IPF.Note", true, 0, 0),
                ews_folder("real", None, "已发送邮件", "IPF.Note", true, 3, 0),
                ews_folder("in", None, "收件箱", "IPF.Note", true, 10, 2),
            ],
            &known,
        );
        let names: Vec<&str> = records.iter().map(|record| record.name.as_str()).collect();
        assert_eq!(
            names,
            ["已发送邮件", "收件箱"],
            "the empty alias is dropped"
        );
        assert_eq!(records[0].role, FolderRole::Sent);
        assert_eq!(records[1].role, FolderRole::Inbox);
    }

    #[test]
    fn empty_look_alike_folders_are_merged_without_distinguished_ids() {
        let records = no_distinguished(&[
            ews_folder("alias", None, "已发送", "IPF.Note", true, 0, 0),
            ews_folder("real", None, "已发送邮件", "IPF.Note", true, 3, 0),
            ews_folder("archive", None, "Archive", "IPF.Note", true, 0, 0),
            ews_folder("archive-cn", None, "存档", "IPF.Note", true, 0, 0),
            ews_folder("in", None, "收件箱", "IPF.Note", true, 48, 2),
        ]);
        let names: Vec<&str> = records.iter().map(|record| record.name.as_str()).collect();
        // Both empty look-alikes and the empty server Archive shell go away;
        // the folder that actually holds mail keeps the Sent role.
        assert_eq!(names, ["已发送邮件", "收件箱"]);
        assert_eq!(records[0].role, FolderRole::Sent);
        assert_eq!(records[1].role, FolderRole::Inbox);
    }

    #[test]
    fn an_alias_with_mail_or_children_is_never_hidden() {
        let records = no_distinguished(&[
            ews_folder("a", None, "已发送", "IPF.Note", true, 2, 0),
            ews_folder("b", None, "已发送邮件", "IPF.Note", true, 3, 0),
            ews_folder("c", None, "Archive", "IPF.Note", true, 1, 0),
            ews_folder("d", None, "存档", "IPF.Note", true, 0, 0),
            ews_folder("kid", Some("d"), "项目", "IPF.Note", true, 0, 0),
        ]);
        assert_eq!(records.len(), 5, "mail and children must stay reachable");
        assert_eq!(records[0].role, FolderRole::Other);
        assert_eq!(records[1].role, FolderRole::Sent);
        assert_eq!(records[3].role, FolderRole::Other);
        assert_eq!(records[4].parent_remote_name.as_deref(), Some("d"));
    }

    #[test]
    fn empty_archive_outbox_and_outlook_shells_leave_one_view_each() {
        // The real sidebar from a localised Exchange mailbox: two archive
        // entries, two "send" entries and a row of Outlook bookkeeping
        // folders, all empty, next to the folders that are actually used.
        let records = no_distinguished(&[
            ews_folder("in", None, "收件箱", "IPF.Note", true, 2, 2),
            ews_folder("drafts", None, "草稿", "IPF.Note", true, 0, 0),
            ews_folder("sent", None, "已发送邮件", "IPF.Note", true, 3, 0),
            ews_folder("archive", None, "Archive", "IPF.Note", true, 0, 0),
            ews_folder("junk", None, "垃圾邮件", "IPF.Note", true, 0, 0),
            ews_folder("trash", None, "已删除邮件", "IPF.Note", true, 0, 0),
            ews_folder("outbox", None, "发件箱", "IPF.Note", true, 0, 0),
            ews_folder("quick", None, "快速步骤设置", "IPF.Note", true, 0, 0),
            ews_folder("files", None, "文件", "IPF.Note", true, 0, 0),
            ews_folder(
                "conv",
                None,
                "Conversation Action Settings",
                "IPF.Note",
                true,
                0,
                0,
            ),
            ews_folder("working", None, "Working Set", "IPF.Note", true, 0, 0),
            ews_folder("sync", None, "同步问题", "IPF.Note", true, 0, 0),
            ews_folder("conflict", Some("sync"), "冲突", "IPF.Note", true, 0, 0),
            ews_folder("yammer", None, "Yammer 根目录", "IPF.Note", true, 0, 0),
            ews_folder("yammer-kid", Some("yammer"), "分组", "IPF.Note", true, 0, 0),
            ews_folder("mine", None, "项目", "IPF.Note", true, 0, 0),
        ]);
        let names: Vec<&str> = records.iter().map(|record| record.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "收件箱",
                "草稿",
                "已发送邮件",
                "垃圾邮件",
                "已删除邮件",
                "同步问题",
                "冲突",
                "Yammer 根目录",
                "分组",
                "项目",
            ]
        );
        // System folders with children keep their subtree intact.
        assert_eq!(records[5].parent_remote_name, None);
        assert_eq!(records[6].parent_remote_name.as_deref(), Some("sync"));
        assert_eq!(records[8].parent_remote_name.as_deref(), Some("yammer"));
    }

    #[test]
    fn a_shell_with_mail_or_children_stays_visible() {
        let records = no_distinguished(&[
            ews_folder("in", None, "收件箱", "IPF.Note", true, 1, 0),
            ews_folder("archive", None, "Archive", "IPF.Note", true, 4, 1),
            ews_folder("outbox", None, "发件箱", "IPF.Note", true, 0, 0),
            ews_folder("queued", Some("outbox"), "待发", "IPF.Note", true, 0, 0),
            ews_folder("quick", None, "快速步骤设置", "IPF.Note", true, 0, 1),
        ]);
        let names: Vec<&str> = records.iter().map(|record| record.name.as_str()).collect();
        assert_eq!(
            names,
            ["收件箱", "Archive", "发件箱", "待发", "快速步骤设置"]
        );
        assert_eq!(records[1].role, FolderRole::Archive);
        assert_eq!(records[3].parent_remote_name.as_deref(), Some("outbox"));
    }

    #[test]
    fn similarly_named_user_folders_are_not_shells() {
        let records = no_distinguished(&[
            ews_folder("in", None, "收件箱", "IPF.Note", true, 1, 0),
            ews_folder("a", None, "Files 2026", "IPF.Note", true, 0, 0),
            ews_folder("b", None, "存档 2026", "IPF.Note", true, 0, 0),
            ews_folder("c", None, "发件箱备份", "IPF.Note", true, 0, 0),
        ]);
        assert_eq!(records.len(), 4, "exact-name matching keeps user folders");
    }

    #[test]
    fn a_partial_getfolder_answer_does_not_shift_roles() {
        assert!(folder_roles_by_id(&[]).is_empty());
        assert!(folder_roles_by_id(&[Some("in".into()), None]).is_empty());
        let known = folder_roles_by_id(&[
            Some("in".into()),
            Some("draft".into()),
            Some("sent".into()),
            None,
            Some("junk".into()),
        ]);
        assert_eq!(known.get("sent"), Some(&FolderRole::Sent));
        assert_eq!(known.get("in"), Some(&FolderRole::Inbox));
        assert_eq!(known.len(), 4);
    }

    #[test]
    fn the_parent_chain_and_unread_counts_reach_the_local_records() {
        let records = no_distinguished(&[
            ews_folder("root", Some("msgroot"), "收件箱", "IPF.Note", true, 12, 3),
            ews_folder("child", Some("root"), "项目", "IPF.Note", true, 7, 1),
            ews_folder("grand", Some("child"), "2026", "IPF.Note", true, 2, 0),
            ews_folder("orphan", Some("gone"), "孤立", "IPF.Note", true, 0, 0),
        ]);
        assert_eq!(records.len(), 4);
        // The message root itself is never returned by FindFolder, so it must
        // not become a dangling parent (replace_folders indexes by remote name).
        assert_eq!(records[0].parent_remote_name, None);
        assert_eq!(records[1].parent_remote_name.as_deref(), Some("root"));
        assert_eq!(records[2].parent_remote_name.as_deref(), Some("child"));
        assert_eq!(records[3].parent_remote_name, None);
        assert_eq!(records[0].status.as_ref().unwrap().unseen, 3);
        assert_eq!(records[0].status.as_ref().unwrap().messages, 12);
        assert_eq!(records[1].status.as_ref().unwrap().unseen, 1);
    }

    #[test]
    fn each_role_is_assigned_once_and_only_near_the_top_level() {
        let records = no_distinguished(&[
            ews_folder("a", None, "收件箱", "IPF.Note", true, 0, 0),
            ews_folder("b", None, "收件箱备份", "IPF.Note", true, 0, 0),
            ews_folder("c", None, "已发送邮件", "IPF.Note", true, 1, 0),
            ews_folder("d", Some("c"), "已发送邮件", "IPF.Note", true, 0, 0),
            ews_folder("e", None, "Sent Items", "IPF.Note", true, 2, 0),
        ]);
        assert_eq!(records.len(), 5);
        assert_eq!(records[0].role, FolderRole::Inbox);
        assert_eq!(
            records[1].role,
            FolderRole::Other,
            "contains() must not guess"
        );
        assert_eq!(records[2].role, FolderRole::Other, "the fuller folder wins");
        assert_eq!(
            records[3].role,
            FolderRole::Other,
            "a nested folder is a user folder"
        );
        assert_eq!(records[4].role, FolderRole::Sent);
    }

    #[test]
    fn calendars_search_folders_and_contacts_are_not_mail_folders() {
        let records = no_distinguished(&[
            ews_folder("in", None, "收件箱", "IPF.Note", true, 0, 0),
            ews_folder("cal", None, "日历", "IPF.Appointment", false, 0, 0),
            ews_folder("search", None, "待处理", "", false, 0, 0),
            ews_folder("custom", None, "自定义", "IPF.Note.MyForm", true, 0, 0),
        ]);
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].name, "收件箱");
        assert_eq!(records[1].name, "自定义", "a custom class stays visible");
    }

    #[test]
    fn ews_dates_become_the_same_format_as_imap_headers() {
        let value = utc(Some("2026-11-12T08:30:00Z"));
        assert_eq!(value, "2026-11-12T08:30:00.000Z");
        assert!(utc(None).ends_with('Z'));
        assert!(utc(Some("not a date")).ends_with('Z'));
    }

    #[test]
    fn a_rejection_is_not_retried_as_a_network_error() {
        let error = ews_failure(EwsError::Http(HttpError::Rejected));
        assert!(matches!(
            error,
            SyncError::Imap(ImapError::Authentication(_))
        ));
    }
}
