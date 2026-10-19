//! Turns a server folder list into local folder records: display names,
//! hierarchy and roles (Inbox, Sent, ...).

use std::collections::{HashMap, HashSet};

use crate::imap::{FolderStatus, RemoteFolder, utf7};
use crate::mail::FolderRole;

/// A folder ready to store, keyed by its server name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FolderRecord {
    pub remote_name: String,
    pub parent_remote_name: Option<String>,
    pub name: String,
    pub delimiter: Option<String>,
    pub role: FolderRole,
    pub selectable: bool,
    pub attributes: Vec<String>,
    pub status: Option<FolderStatus>,
}

/// SPECIAL-USE attributes (RFC 6154) that map to a role.
const SPECIAL_USE: [(&str, FolderRole); 5] = [
    ("\\Sent", FolderRole::Sent),
    ("\\Drafts", FolderRole::Drafts),
    ("\\Trash", FolderRole::Trash),
    ("\\Junk", FolderRole::Junk),
    ("\\Archive", FolderRole::Archive),
];

/// Common names for servers without SPECIAL-USE, compared case-insensitively.
const WELL_KNOWN_NAMES: [(FolderRole, &[&str]); 5] = [
    (
        FolderRole::Sent,
        &[
            "sent",
            "sent messages",
            "sent items",
            "sent mail",
            "已发送",
            "已发送邮件",
            "已发邮件",
        ],
    ),
    (FolderRole::Drafts, &["drafts", "draft", "草稿", "草稿箱"]),
    (
        FolderRole::Trash,
        &[
            "trash",
            "deleted",
            "deleted messages",
            "deleted items",
            "bin",
            "已删除",
            "已删除邮件",
            "废纸篓",
            "回收站",
        ],
    ),
    (
        FolderRole::Junk,
        &[
            "junk",
            "spam",
            "junk e-mail",
            "junk email",
            "垃圾邮件",
            "垃圾箱",
        ],
    ),
    (FolderRole::Archive, &["archive", "archives", "归档"]),
];

fn is_inbox(name: &str) -> bool {
    name.eq_ignore_ascii_case("INBOX")
}

fn has_attribute(folder: &RemoteFolder, wanted: &str) -> bool {
    folder
        .attributes
        .iter()
        .any(|attribute| attribute.eq_ignore_ascii_case(wanted))
}

fn is_selectable(folder: &RemoteFolder) -> bool {
    // LIST establishes mailbox capability. STATUS is a later, independent
    // counts observation and may not have succeeded yet.
    !has_attribute(folder, "\\Noselect") && !has_attribute(folder, "\\NonExistent")
}

/// Server name of the parent folder, when the list contains it.
fn parent_name(folder: &RemoteFolder, names: &HashSet<&str>) -> Option<String> {
    let delimiter = folder
        .delimiter
        .as_deref()
        .filter(|delimiter| !delimiter.is_empty())?;
    let (parent, _) = folder.name.rsplit_once(delimiter)?;
    names.contains(parent).then(|| parent.to_owned())
}

fn last_segment(folder: &RemoteFolder) -> &str {
    match folder
        .delimiter
        .as_deref()
        .filter(|delimiter| !delimiter.is_empty())
    {
        Some(delimiter) => folder.name.rsplit(delimiter).next().unwrap_or(&folder.name),
        None => &folder.name,
    }
}

/// Assigns each role to at most one folder: INBOX by name, then SPECIAL-USE
/// attributes, then well-known names for roles still missing. Name guessing
/// only applies at the top level or directly under INBOX (`INBOX.Sent`).
pub(crate) fn interpret(remote: &[RemoteFolder]) -> Vec<FolderRecord> {
    let names: HashSet<&str> = remote.iter().map(|folder| folder.name.as_str()).collect();
    let parents: Vec<Option<String>> = remote
        .iter()
        .map(|folder| parent_name(folder, &names))
        .collect();
    let display: Vec<String> = remote
        .iter()
        .map(|folder| {
            let segment = last_segment(folder);
            utf7::decode(segment).unwrap_or_else(|| segment.to_owned())
        })
        .collect();

    let mut roles: HashMap<usize, FolderRole> = HashMap::new();
    let mut taken: HashSet<FolderRole> = HashSet::new();
    let mut assign = |index: usize, role: FolderRole, roles: &mut HashMap<usize, FolderRole>| {
        if !roles.contains_key(&index) && taken.insert(role) {
            roles.insert(index, role);
        }
    };
    for (index, folder) in remote.iter().enumerate() {
        if is_selectable(folder) && is_inbox(&folder.name) {
            assign(index, FolderRole::Inbox, &mut roles);
        }
    }
    for (index, folder) in remote.iter().enumerate() {
        if !is_selectable(folder) {
            continue;
        }
        for (attribute, role) in SPECIAL_USE {
            if has_attribute(folder, attribute) {
                assign(index, role, &mut roles);
            }
        }
    }
    for (index, folder) in remote.iter().enumerate() {
        let near_top = parents[index].as_deref().is_none_or(is_inbox);
        if !near_top || !is_selectable(folder) {
            continue;
        }
        let name = display[index].to_lowercase();
        for (role, candidates) in WELL_KNOWN_NAMES {
            if candidates.contains(&name.as_str()) {
                assign(index, role, &mut roles);
            }
        }
    }

    remote
        .iter()
        .enumerate()
        .map(|(index, folder)| {
            let role = roles.get(&index).copied().unwrap_or(FolderRole::Other);
            // System folders under INBOX (Courier-style `INBOX.Sent`) sit beside it, not inside.
            let parent = parents[index]
                .clone()
                .filter(|parent| !(is_inbox(parent) && role != FolderRole::Other));
            FolderRecord {
                remote_name: folder.name.clone(),
                parent_remote_name: parent,
                name: display[index].clone(),
                delimiter: folder.delimiter.clone(),
                role,
                selectable: is_selectable(folder),
                attributes: folder.attributes.clone(),
                status: folder.status,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str, delimiter: &str, attributes: &[&str]) -> RemoteFolder {
        RemoteFolder {
            name: name.into(),
            delimiter: Some(delimiter.into()),
            attributes: attributes
                .iter()
                .map(|attribute| (*attribute).into())
                .collect(),
            status: None,
        }
    }

    fn roles(records: &[FolderRecord]) -> Vec<(&str, FolderRole)> {
        records
            .iter()
            .map(|record| (record.name.as_str(), record.role))
            .collect()
    }

    #[test]
    fn special_use_attributes_win_over_names() {
        let records = interpret(&[
            folder("INBOX", "/", &[]),
            folder("Sent", "/", &[]),
            folder("&XfJT0ZAB-", "/", &["\\Sent"]),
            folder("Junk", "/", &["\\HasNoChildren", "\\junk"]),
        ]);
        assert_eq!(
            roles(&records),
            [
                ("INBOX", FolderRole::Inbox),
                ("Sent", FolderRole::Other),
                ("已发送", FolderRole::Sent),
                ("Junk", FolderRole::Junk)
            ]
        );
    }

    #[test]
    fn guesses_roles_from_names_when_special_use_is_missing() {
        let records = interpret(&[
            folder("inbox", "/", &[]),
            folder("Sent Messages", "/", &[]),
            folder("&g0l6P3ux-", "/", &[]),
            folder("&XfJSIJZk-", "/", &[]),
            folder("Projects", "/", &[]),
            folder("Projects/Trash", "/", &[]),
        ]);
        assert_eq!(
            roles(&records),
            [
                ("inbox", FolderRole::Inbox),
                ("Sent Messages", FolderRole::Sent),
                ("草稿箱", FolderRole::Drafts),
                ("已删除", FolderRole::Trash),
                ("Projects", FolderRole::Other),
                ("Trash", FolderRole::Other),
            ],
            "a nested folder named Trash is a user folder"
        );
    }

    #[test]
    fn builds_hierarchy_from_the_delimiter() {
        let records = interpret(&[
            folder("INBOX", ".", &[]),
            folder("INBOX.Sent", ".", &[]),
            folder("INBOX.Receipts", ".", &[]),
            folder("Clients", ".", &["\\Noselect"]),
            folder("Clients.East", ".", &[]),
            folder("Orphan.Child", ".", &[]),
        ]);
        let parents: Vec<_> = records
            .iter()
            .map(|record| record.parent_remote_name.as_deref())
            .collect();
        assert_eq!(
            parents,
            [None, None, Some("INBOX"), None, Some("Clients"), None]
        );
        assert_eq!(
            records[1].role,
            FolderRole::Sent,
            "INBOX.Sent is lifted beside the inbox"
        );
        assert!(!records[3].selectable);
        assert_eq!(records[4].name, "East");
        assert_eq!(
            records[5].name, "Child",
            "a missing parent keeps the folder at the top level"
        );
    }

    #[test]
    fn each_role_goes_to_one_folder() {
        let records = interpret(&[
            folder("Trash", "/", &["\\Trash"]),
            folder("Deleted Items", "/", &["\\Trash"]),
        ]);
        assert_eq!(
            roles(&records),
            [
                ("Trash", FolderRole::Trash),
                ("Deleted Items", FolderRole::Other)
            ]
        );
    }

    #[test]
    fn list_only_containers_do_not_claim_selectable_folder_roles() {
        let records = interpret(&[
            folder("Legacy", "/", &["\\noselect", "\\Sent"]),
            folder("Sent", "/", &[]),
            folder("Missing", "/", &["\\nonexistent", "\\Trash"]),
            folder("Trash", "/", &[]),
        ]);
        assert_eq!(
            roles(&records),
            [
                ("Legacy", FolderRole::Other),
                ("Sent", FolderRole::Sent),
                ("Missing", FolderRole::Other),
                ("Trash", FolderRole::Trash),
            ]
        );
        assert_eq!(
            records
                .iter()
                .map(|record| record.selectable)
                .collect::<Vec<_>>(),
            [false, true, false, true]
        );
        assert!(records.iter().all(|record| record.status.is_none()));
    }

    #[test]
    fn keeps_undecodable_names_as_sent_by_the_server() {
        assert_eq!(
            interpret(&[folder("Bad&Name", "/", &[])])[0].name,
            "Bad&Name"
        );
    }
}
