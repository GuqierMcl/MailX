//! SOAP envelopes and the small, bounded reader that goes with them.
//!
//! Responses come from a server, so nothing here trusts them: a `DOCTYPE` is
//! refused outright (no DTD processing means no entity expansion), every read is
//! an offset-bounded slice, and the only shapes understood are the ones this
//! module asks for. Element prefixes (`t:`, `m:`) are matched by local name
//! because Exchange and its load balancers do not always agree on them.

use base64::{Engine, engine::general_purpose::STANDARD};

/// Message namespace, used for the body element of each request.
const MESSAGES: &str = "http://schemas.microsoft.com/exchange/services/2006/messages";
/// Type namespace, used for every value type.
const TYPES: &str = "http://schemas.microsoft.com/exchange/services/2006/types";
/// The schema version MailX speaks. Exchange 2013 answers this and newer servers
/// accept it too.
const SERVER_VERSION: &str = "Exchange2013";
/// A decoded MIME message larger than this is refused instead of returned.
const MAX_MIME_BYTES: usize = 32 * 1024 * 1024;
/// `t:Item` subclasses this reader treats as mail items.
const ITEM_ELEMENTS: [&str; 6] = [
    "Message",
    "Item",
    "MeetingMessage",
    "MeetingRequest",
    "MeetingResponse",
    "MeetingCancellation",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SoapError {
    /// The response carries a DTD or is not something this reader accepts.
    Malformed,
    /// `ResponseClass="Error"`; the message stays on the server side.
    Server,
}

/// One folder as `FindFolder` reports it. A deep traversal returns a flat list,
/// so the parent id and the folder class are what let the caller rebuild the
/// tree and tell mail folders from calendars, contacts and search folders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Folder {
    pub id: String,
    pub parent_id: Option<String>,
    pub display_name: String,
    pub total_count: u32,
    pub unread_count: u32,
    /// `ChildFolderCount` is part of `BaseShape=Default`; it tells us whether a
    /// shallow follow-up is worth issuing when a server omits parent ids.
    pub child_folder_count: Option<u32>,
    /// `IPF.Note` for mail folders; absent on servers that omit the property.
    pub folder_class: Option<String>,
    /// False for calendar / contacts / tasks / search folder elements.
    pub mail_folder: bool,
}

/// One page of a `FindFolder` listing. `next_offset` is `None` once the server
/// says the last item was included, or when paging cannot advance any further.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FoldersPage {
    pub folders: Vec<Folder>,
    pub next_offset: Option<usize>,
}

/// One message as FindItem reports it. Addresses are already decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Party {
    pub name: Option<String>,
    pub address: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Item {
    pub id: String,
    pub change_key: Option<String>,
    pub subject: String,
    pub from: Option<Party>,
    pub to: Vec<Party>,
    pub cc: Vec<Party>,
    pub reply_to: Vec<Party>,
    /// RFC 3339 / ISO 8601 strings straight from EWS; caller parses them.
    pub sent_at: Option<String>,
    pub received_at: Option<String>,
    pub is_read: bool,
    pub is_flagged: bool,
    pub has_attachments: bool,
    pub size: Option<u32>,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
}

/// Wraps a request body in an envelope naming the schema version.
pub(crate) fn envelope(body: &str) -> String {
    format!(
        concat!(
            r#"<?xml version="1.0" encoding="utf-8"?>"#,
            r#"<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/" "#,
            r#"xmlns:m="{messages}" xmlns:t="{types}">"#,
            r#"<soap:Header><t:RequestServerVersion Version="{version}"/></soap:Header>"#,
            r#"<soap:Body>{body}</soap:Body></soap:Envelope>"#,
        ),
        messages = MESSAGES,
        types = TYPES,
        version = SERVER_VERSION,
        body = body,
    )
}

/// `FindFolder` rooted at the message-folder root, deep, one page of 256.
/// This is also the body NTLM Type 3 carries, so it deliberately uses the bare
/// `Default` shape: an extra `FieldURI` rejected during schema validation would
/// turn a successful sign-in into an HTTP 500 (see `items()` for the same trap).
pub(crate) fn find_folders() -> String {
    find_folders_plain(0)
}

/// `BaseShape=Default` returns only `FolderId`, `DisplayName`, `UnreadCount`,
/// `TotalCount` and `ChildFolderCount` for a folder. `ParentFolderId` and
/// `FolderClass` are NOT in that set: they have to be asked for explicitly or
/// the flat list of a deep traversal cannot be turned back into a tree.
/// All three are members of `UnindexedFieldURIType` (MS-OXWSCDATA 2.2.5.26).
const FOLDER_FIELDS: &[&str] = &[
    "folder:ParentFolderId",
    "folder:FolderClass",
    "folder:UnreadCount",
];

const DEFAULT_SHAPE: &str = r#"<t:BaseShape>Default</t:BaseShape>"#;
const ALL_SHAPE: &str = r#"<t:BaseShape>AllProperties</t:BaseShape>"#;
const ROOT_FOLDER: &str = r#"<t:DistinguishedFolderId Id="msgfolderroot"/>"#;

/// The `offset`-th page of the deep listing, explicitly asking for the parent
/// id and the folder class. A strictly validating server that rejects the
/// extra `FieldURI` paths falls back through [`find_folders_all`] and
/// [`find_folders_plain`].
pub(crate) fn find_folders_page(offset: usize) -> String {
    let mut shape = String::from(DEFAULT_SHAPE);
    shape.push_str("<t:AdditionalProperties>");
    for field in FOLDER_FIELDS {
        debug_assert!(
            ALLOWED_FIELD_URIS.contains(field),
            "{field} is not a valid EWS FieldURI"
        );
        shape.push_str(&format!(r#"<t:FieldURI FieldURI="{field}"/>"#));
    }
    shape.push_str("</t:AdditionalProperties>");
    find_folders_with(offset, &shape, "Deep", ROOT_FOLDER)
}

/// The same page with every schema property. Used when a deployment rejects
/// `AdditionalProperties` outright: `AllProperties` still carries the parent.
pub(crate) fn find_folders_all(offset: usize) -> String {
    find_folders_with(offset, ALL_SHAPE, "Deep", ROOT_FOLDER)
}

/// The same page with the bare `Default` shape, for servers that refuse any
/// richer shape. The reader treats missing parent / class / unread as absent.
pub(crate) fn find_folders_plain(offset: usize) -> String {
    find_folders_with(offset, DEFAULT_SHAPE, "Deep", ROOT_FOLDER)
}

/// Direct children of one known folder. The caller already knows the parent,
/// so this rebuilds the hierarchy without depending on `ParentFolderId` in the
/// response at all.
pub(crate) fn find_folder_children(parent_id: &str, offset: usize) -> String {
    let parent = format!(r#"<t:FolderId Id="{}"/>"#, escape(parent_id));
    find_folders_with(offset, DEFAULT_SHAPE, "Shallow", &parent)
}

/// One `GetFolder` for several distinguished folders. The response messages
/// come back in request order, which is how the caller maps ids to roles.
/// `IdOnly` keeps the answer small. A mailbox can contain empty folders whose
/// names look exactly like the default ones after a locale switch; only this
/// operation can say which folder really receives sent mail, drafts and so on.
pub(crate) fn get_distinguished_folders(names: &[&str]) -> String {
    let ids: String = names
        .iter()
        .map(|name| format!(r#"<t:DistinguishedFolderId Id="{}"/>"#, escape(name)))
        .collect();
    envelope(&format!(
        concat!(
            r#"<m:GetFolder>"#,
            r#"<m:FolderShape><t:BaseShape>IdOnly</t:BaseShape></m:FolderShape>"#,
            r#"<m:FolderIds>{ids}</m:FolderIds>"#,
            r#"</m:GetFolder>"#,
        ),
        ids = ids,
    ))
}

/// Folder ids from a `GetFolder` response, one slot per requested folder.
/// A failed response message leaves its slot empty so the remaining ids stay
/// aligned with the request. Anything unreadable yields an empty list; the
/// caller treats that as "guess roles from names" rather than an error.
pub(crate) fn parse_distinguished_folders(xml: &str) -> Vec<Option<String>> {
    let Ok(xml) = checked(xml) else {
        return Vec::new();
    };
    blocks(xml, "GetFolderResponseMessage")
        .iter()
        .map(|block| {
            if attribute(block, "ResponseClass").is_some_and(|class| class != "Success") {
                return None;
            }
            blocks(block, "FolderId")
                .first()
                .copied()
                .and_then(|tag| attribute(tag, "Id"))
        })
        .collect()
}

fn find_folders_with(offset: usize, shape: &str, traversal: &str, parents: &str) -> String {
    envelope(&format!(
        concat!(
            r#"<m:FindFolder Traversal="{traversal}">"#,
            r#"<m:FolderShape>{shape}</m:FolderShape>"#,
            r#"<m:IndexedPageFolderView MaxEntriesReturned="256" Offset="{offset}" BasePoint="Beginning"/>"#,
            r#"<m:ParentFolderIds>{parents}</m:ParentFolderIds>"#,
            r#"</m:FindFolder>"#,
        ),
        shape = shape,
        traversal = traversal,
        offset = offset,
        parents = parents,
    ))
}

/// Refuses a DTD: without entity expansion the reader cannot be tricked into
/// walking anything a server smuggles in.
fn checked_dtd(xml: &str) -> Result<&str, SoapError> {
    let upper = xml.to_ascii_uppercase();
    if upper.contains("<!DOCTYPE") || upper.contains("<!ENTITY") {
        return Err(SoapError::Malformed);
    }
    Ok(xml)
}

/// Rejects anything this reader must not walk: a DTD, or a body that is not an
/// envelope at all.
fn checked(xml: &str) -> Result<&str, SoapError> {
    let xml = checked_dtd(xml)?;
    if !xml.to_ascii_uppercase().contains("ENVELOPE") {
        return Err(SoapError::Malformed);
    }
    Ok(xml)
}

/// Reads the folder list out of a `FindFolder` response.
pub(crate) fn parse_folders(xml: &str) -> Result<Vec<Folder>, SoapError> {
    parse_folders_page(xml, 0).map(|page| page.folders)
}

/// Reads one page together with the paging cursor on `RootFolder`.
///
/// A deep traversal is a flat list: `ParentFolderId` and `FolderClass` are what
/// the caller needs to rebuild the tree and skip non-mail folders. Elements are
/// collected in document order, not grouped by their XML element name, so the
/// local sidebar keeps the server's ordering.
pub(crate) fn parse_folders_page(xml: &str, offset: usize) -> Result<FoldersPage, SoapError> {
    let xml = checked(xml)?;
    // A per-message failure is reported inside a 200, so the class decides.
    if let Some(message) = blocks(xml, "FindFolderResponseMessage").first()
        && attribute(message, "ResponseClass").is_some_and(|class| class != "Success")
    {
        return Err(SoapError::Server);
    }
    let Some(root) = blocks(xml, "RootFolder").first().copied() else {
        return Err(SoapError::Malformed);
    };
    // Exchange and its load balancers disagree on prefixes; local names only.
    const KINDS: [(&str, bool); 5] = [
        ("Folder", true),
        ("CalendarFolder", false),
        ("ContactsFolder", false),
        ("TasksFolder", false),
        ("SearchFolder", false),
    ];
    let base = root.as_ptr() as usize;
    let mut found: Vec<(usize, &str, bool)> = Vec::new();
    for (name, mail_folder) in KINDS {
        for block in blocks(root, name) {
            found.push((block.as_ptr() as usize - base, block, mail_folder));
        }
    }
    found.sort_by_key(|(position, _, _)| *position);
    let mut folders = Vec::new();
    for (_, block, mail_folder) in found {
        let Some(id_tag) = blocks(block, "FolderId").first().copied() else {
            continue;
        };
        let Some(id) = attribute(id_tag, "Id") else {
            continue;
        };
        folders.push(Folder {
            id,
            parent_id: blocks(block, "ParentFolderId")
                .first()
                .copied()
                .and_then(|tag| attribute(tag, "Id")),
            display_name: text(block, "DisplayName").unwrap_or_default(),
            total_count: text(block, "TotalCount")
                .and_then(|count| count.trim().parse().ok())
                .unwrap_or(0),
            unread_count: text(block, "UnreadCount")
                .and_then(|count| count.trim().parse().ok())
                .unwrap_or(0),
            child_folder_count: text(block, "ChildFolderCount")
                .and_then(|count| count.trim().parse().ok()),
            folder_class: text(block, "FolderClass").filter(|class| !class.is_empty()),
            mail_folder,
        });
    }
    // `IncludesLastItemInRange` is on RootFolder, not on each folder. Servers
    // that omit it are treated as a single page, which is the old behaviour.
    let last = attribute(root, "IncludesLastItemInRange")
        .is_none_or(|value| !value.eq_ignore_ascii_case("false"));
    let next_offset = if last {
        None
    } else {
        let reported = attribute(root, "IndexedPagingOffset")
            .and_then(|value| value.trim().parse::<usize>().ok());
        let next = reported.unwrap_or_else(|| offset.saturating_add(folders.len()));
        // A page that cannot advance would loop forever; an empty page ends it.
        (next > offset && !folders.is_empty()).then_some(next)
    };
    Ok(FoldersPage {
        folders,
        next_offset,
    })
}

/// Every `FieldURI` value MailX is allowed to emit must be a member of the EWS
/// `UnindexedFieldURIType` enumeration. A value outside it is not a per-item
/// error: Exchange fails while deserializing the request and answers with a
/// top-level SOAP fault (`ErrorInvalidRequest`, "请求无效") over HTTP 500.
/// `InReplyTo` belongs to `item:`, not `message:` — see the enumeration in
/// MS-OXWSCDATA 2.2.5.26 / `UnindexedFieldURIType`.
const ALLOWED_FIELD_URIS: &[&str] = &[
    "item:Subject",
    "message:From",
    "message:ToRecipients",
    "message:CcRecipients",
    "message:ReplyTo",
    "item:DateTimeSent",
    "item:DateTimeReceived",
    "message:IsRead",
    "item:HasAttachments",
    "item:Size",
    "message:InternetMessageId",
    "item:InReplyTo",
    "message:References",
    "item:Flag",
    // FolderShape properties (FindFolder). These are legal members too; an
    // unknown value would make Exchange reject the whole request with a 500.
    "folder:ParentFolderId",
    "folder:FolderClass",
    "folder:UnreadCount",
];

/// Fields present since the original EWS schema. `item:Flag`, `item:InReplyTo`,
/// `message:References` and `message:ReplyTo` arrived in Exchange 2010 SP2, so
/// older or strictly validating servers can still reject the full set. A
/// rejected `FindItem` retries with this conservative set instead of failing
/// the whole account; the reader already defaults every missing field.
const ITEM_FIELDS: &[&str] = &[
    "item:Subject",
    "message:From",
    "message:ToRecipients",
    "message:CcRecipients",
    "message:ReplyTo",
    "item:DateTimeSent",
    "item:DateTimeReceived",
    "message:IsRead",
    "item:HasAttachments",
    "item:Size",
    "message:InternetMessageId",
    "item:InReplyTo",
    "message:References",
    "item:Flag",
];

const BASIC_ITEM_FIELDS: &[&str] = &[
    "item:Subject",
    "message:From",
    "message:ToRecipients",
    "message:CcRecipients",
    "item:DateTimeSent",
    "item:DateTimeReceived",
    "message:IsRead",
    "item:HasAttachments",
    "item:Size",
    "message:InternetMessageId",
];

/// `FindItem` in one folder, shallow, one page of `max_entries` messages.
pub(crate) fn find_items(folder_id: &str, max_entries: usize) -> String {
    find_items_with(folder_id, max_entries, ITEM_FIELDS)
}

/// The same page using only universally supported property paths, for servers
/// that reject one of the newer fields in [`ITEM_FIELDS`].
pub(crate) fn find_items_basic(folder_id: &str, max_entries: usize) -> String {
    find_items_with(folder_id, max_entries, BASIC_ITEM_FIELDS)
}

fn find_items_with(folder_id: &str, max_entries: usize, fields: &[&str]) -> String {
    let folder = escape(folder_id);
    let mut properties = String::new();
    for field in fields {
        debug_assert!(
            ALLOWED_FIELD_URIS.contains(field),
            "{field} is not a valid EWS FieldURI"
        );
        properties.push_str(&format!(r#"<t:FieldURI FieldURI="{field}"/>"#));
    }
    envelope(&format!(
        concat!(
            r#"<m:FindItem Traversal="Shallow">"#,
            r#"<m:ItemShape>"#,
            r#"<t:BaseShape>IdOnly</t:BaseShape>"#,
            r#"<t:AdditionalProperties>{properties}</t:AdditionalProperties>"#,
            r#"</m:ItemShape>"#,
            r#"<m:IndexedPageItemView MaxEntriesReturned="{max_entries}" Offset="0" BasePoint="Beginning"/>"#,
            r#"<m:ParentFolderIds><t:FolderId Id="{folder}"/></m:ParentFolderIds>"#,
            r#"</m:FindItem>"#,
        ),
        properties = properties,
        max_entries = max_entries,
        folder = folder,
    ))
}

/// Reads the messages out of a `FindItem` response.
pub(crate) fn parse_items(xml: &str) -> Result<Vec<Item>, SoapError> {
    let xml = checked(xml)?;
    // A per-message failure is reported inside a 200, so the class decides.
    if has_server_error(xml, "FindItemResponseMessage") {
        return Err(SoapError::Server);
    }
    let Some(root) = blocks(xml, "RootFolder").first().copied() else {
        return Err(SoapError::Malformed);
    };
    let mut items = Vec::new();
    for name in ITEM_ELEMENTS {
        for block in blocks(root, name) {
            items.push(parse_item(block));
        }
    }
    Ok(items)
}

/// `GetItem` asking for the raw MIME content of one message.
pub(crate) fn get_item_mime(item_id: &str, change_key: Option<&str>) -> String {
    let item = item_id_tag(item_id, change_key);
    envelope(&format!(
        concat!(
            r#"<m:GetItem>"#,
            r#"<m:ItemShape>"#,
            r#"<t:BaseShape>IdOnly</t:BaseShape>"#,
            r#"<t:IncludeMimeContent>true</t:IncludeMimeContent>"#,
            r#"</m:ItemShape>"#,
            r#"<m:ItemIds>{item}</m:ItemIds>"#,
            r#"</m:GetItem>"#,
        ),
        item = item,
    ))
}

/// Decodes `<t:MimeContent>` base64. Refuses empty or oversized payloads (>32 MiB).
pub(crate) fn parse_mime(xml: &str) -> Result<Vec<u8>, SoapError> {
    let xml = checked(xml)?;
    if has_server_error(xml, "GetItemResponseMessage") {
        return Err(SoapError::Server);
    }
    let Some(raw) = raw_text(xml, "MimeContent") else {
        return Err(SoapError::Malformed);
    };
    // The transport already bounds the response; this also guards callers that
    // hand us a larger string directly, before any base64 decoding.
    if raw.len() > MAX_MIME_BYTES {
        return Err(SoapError::Malformed);
    }
    // Exchange wraps base64 across lines; whitespace is not part of the data.
    let compact: String = raw
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    if compact.is_empty() {
        return Err(SoapError::Malformed);
    }
    let decoded = STANDARD
        .decode(compact.as_bytes())
        .map_err(|_| SoapError::Malformed)?;
    if decoded.is_empty() || decoded.len() > MAX_MIME_BYTES {
        return Err(SoapError::Malformed);
    }
    Ok(decoded)
}

/// `UpdateItem` writing back the read / flag state of one message.
pub(crate) fn update_flags(
    item_id: &str,
    change_key: Option<&str>,
    read: Option<bool>,
    flagged: Option<bool>,
) -> String {
    let mut updates = String::new();
    if let Some(read) = read {
        let value = if read { "true" } else { "false" };
        updates.push_str(&format!(
            concat!(
                r#"<t:SetItemField>"#,
                r#"<t:FieldURI FieldURI="message:IsRead"/>"#,
                r#"<t:Message><t:IsRead>{value}</t:IsRead></t:Message>"#,
                r#"</t:SetItemField>"#,
            ),
            value = value
        ));
    }
    if let Some(flagged) = flagged {
        let value = if flagged { "Flagged" } else { "NotFlagged" };
        updates.push_str(&format!(
            concat!(
                r#"<t:SetItemField>"#,
                r#"<t:FieldURI FieldURI="item:Flag"/>"#,
                r#"<t:Message><t:Flag><t:FlagStatus>{value}</t:FlagStatus></t:Flag></t:Message>"#,
                r#"</t:SetItemField>"#,
            ),
            value = value
        ));
    }
    let item = item_id_tag(item_id, change_key);
    envelope(&format!(
        concat!(
            r#"<m:UpdateItem MessageDisposition="SaveOnly" ConflictResolution="AutoResolve">"#,
            r#"<m:ItemChanges><t:ItemChange>"#,
            r#"{item}"#,
            r#"<t:Updates>{updates}</t:Updates>"#,
            r#"</t:ItemChange></m:ItemChanges>"#,
            r#"</m:UpdateItem>"#,
        ),
        item = item,
        updates = updates,
    ))
}

/// `DeleteItem` moving one message to Deleted Items.
pub(crate) fn delete_item(item_id: &str, change_key: Option<&str>) -> String {
    let item = item_id_tag(item_id, change_key);
    envelope(&format!(
        concat!(
            r#"<m:DeleteItem DeleteType="MoveToDeletedItems" SendMeetingCancellations="None">"#,
            r#"<m:ItemIds>{item}</m:ItemIds>"#,
            r#"</m:DeleteItem>"#,
        ),
        item = item,
    ))
}

/// CreateItem MessageDisposition="SendAndSaveCopy" with a raw MIME message.
pub(crate) fn create_and_send(mime: &[u8]) -> String {
    let encoded = STANDARD.encode(mime);
    envelope(&format!(
        concat!(
            r#"<m:CreateItem MessageDisposition="SendAndSaveCopy">"#,
            r#"<m:SavedItemFolderId><t:DistinguishedFolderId Id="sentitems"/></m:SavedItemFolderId>"#,
            r#"<m:Items>"#,
            r#"<t:Message>"#,
            r#"<t:MimeContent CharacterSet="UTF-8">{encoded}</t:MimeContent>"#,
            r#"</t:Message>"#,
            r#"</m:Items>"#,
            r#"</m:CreateItem>"#,
        ),
        encoded = encoded,
    ))
}

/// Checks the ResponseClass / ResponseCode of one action's response.
pub(crate) fn response_ok(xml: &str, response_message: &str) -> Result<(), SoapError> {
    let xml = checked_dtd(xml)?;
    let Some(block) = blocks(xml, response_message).first().copied() else {
        return Err(SoapError::Malformed);
    };
    if attribute(block, "ResponseClass").is_some_and(|class| class != "Success") {
        return Err(SoapError::Server);
    }
    if text(block, "ResponseCode").is_some_and(|code| code.trim() != "NoError") {
        return Err(SoapError::Server);
    }
    Ok(())
}

/// The element text of `<…:name>` in `xml`, first match, entities decoded.
fn text(xml: &str, name: &str) -> Option<String> {
    raw_text(xml, name).map(unescape)
}

/// The raw element text of `<…:name>` in `xml`, first match.
fn raw_text<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    let block = blocks(xml, name).into_iter().next()?;
    let open_end = block.find('>')? + 1;
    let close = block.rfind("</")?;
    (open_end <= close).then(|| &block[open_end..close])
}

/// Every `<…:name …>…</…:name>` block in `xml`, outermost first.
fn blocks<'a>(xml: &'a str, name: &str) -> Vec<&'a str> {
    let mut found = Vec::new();
    let mut rest = xml;
    while let Some(start) = open_tag(rest, name) {
        let candidate = &rest[start..];
        // A self-closing element is its own block: `<t:FolderId …/>` has no end tag.
        if let Some(end) = self_closing(candidate) {
            found.push(&candidate[..end]);
            rest = &candidate[end..];
            continue;
        }
        let Some(end) = close_tag(candidate, name) else {
            break;
        };
        found.push(&candidate[..end]);
        rest = &candidate[end..];
    }
    found
}

/// Byte offset of `<prefix:name` or `<name` whose tag ends next, if any.
fn open_tag(xml: &str, name: &str) -> Option<usize> {
    let mut offset = 0;
    while let Some(found) = xml[offset..].find('<') {
        let start = offset + found;
        let after = &xml[start + 1..];
        // Skip comments, processing instructions and closing tags.
        if after.starts_with('!') || after.starts_with('?') || after.starts_with('/') {
            offset = start + 1;
            continue;
        }
        let tag_end = after.find(is_tag_end)?;
        let tag = &after[..tag_end];
        let local = tag.rsplit(':').next().unwrap_or(tag);
        let following = xml[start + 1 + tag_end..].chars().next();
        if local == name && matches!(following, Some('>' | ' ' | '/')) {
            return Some(start);
        }
        offset = start + 1;
    }
    None
}

/// Byte offset just past `</…:name>` for the element starting at `xml[0]`.
fn close_tag(xml: &str, name: &str) -> Option<usize> {
    let mut offset = 0;
    while let Some(found) = xml[offset..].find("</") {
        let start = offset + found;
        let after = &xml[start + 2..];
        let tag_end = after.find('>')?;
        let tag = after[..tag_end].trim();
        let local = tag.rsplit(':').next().unwrap_or(tag);
        if local == name {
            return Some(start + 2 + tag_end + 1);
        }
        offset = start + 2;
    }
    None
}

/// Length of the tag at `xml[0]` when it closes itself, `None` otherwise.
fn self_closing(xml: &str) -> Option<usize> {
    let end = xml.find('>')?;
    xml[..end].trim_end().ends_with('/').then_some(end + 1)
}

fn is_tag_end(character: char) -> bool {
    matches!(character, '>' | ' ' | '/' | '\n' | '\r' | '\t')
}

/// The value of `attribute` on the first tag of `block`. The name must start a
/// word, so `Id` never matches the tail of `FolderId`.
fn attribute(block: &str, attribute: &str) -> Option<String> {
    let end = block.find('>')?;
    let tag = &block[..end];
    let needle = format!(" {attribute}=");
    let start = tag.find(&needle)? + needle.len();
    let value = &tag[start..];
    let quote = value.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let body = &value[1..];
    let close = body.find(quote)?;
    Some(unescape(&body[..close]))
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Escapes a value before it is written into XML. Order matters: `&` first.
fn escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// `<t:ItemId Id="…" ChangeKey="…"/>`; the ChangeKey is optional.
fn item_id_tag(item_id: &str, change_key: Option<&str>) -> String {
    let id = escape(item_id);
    match change_key {
        Some(key) => {
            let key = escape(key);
            format!(r#"<t:ItemId Id="{id}" ChangeKey="{key}"/>"#)
        }
        None => format!(r#"<t:ItemId Id="{id}"/>"#),
    }
}

/// `true` when a `…ResponseMessage` is present but reports a failure.
fn has_server_error(xml: &str, message: &str) -> bool {
    blocks(xml, message)
        .iter()
        .any(|block| attribute(block, "ResponseClass").is_some_and(|class| class != "Success"))
}

/// Every `<t:Mailbox>` inside a recipient container, as decoded parties.
fn parse_parties(container: &str) -> Vec<Party> {
    let mut parties = Vec::new();
    for mailbox in blocks(container, "Mailbox") {
        let Some(address) = text(mailbox, "EmailAddress") else {
            continue;
        };
        parties.push(Party {
            name: text(mailbox, "Name").filter(|name| !name.is_empty()),
            address,
        });
    }
    parties
}

/// The first `<…:name>` container, parsed as a recipient list.
fn field_parties(block: &str, name: &str) -> Vec<Party> {
    blocks(block, name)
        .first()
        .copied()
        .map(parse_parties)
        .unwrap_or_default()
}

/// One `t:Item` subclass block into an `Item`; missing fields take defaults.
fn parse_item(block: &str) -> Item {
    let (id, change_key) = blocks(block, "ItemId")
        .first()
        .copied()
        .map(|id| {
            (
                attribute(id, "Id").unwrap_or_default(),
                attribute(id, "ChangeKey"),
            )
        })
        .unwrap_or_default();
    Item {
        id,
        change_key,
        subject: text(block, "Subject").unwrap_or_default(),
        from: field_parties(block, "From").into_iter().next(),
        to: field_parties(block, "ToRecipients"),
        cc: field_parties(block, "CcRecipients"),
        reply_to: field_parties(block, "ReplyTo"),
        sent_at: text(block, "DateTimeSent"),
        received_at: text(block, "DateTimeReceived"),
        is_read: text(block, "IsRead")
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("true")),
        is_flagged: text(block, "FlagStatus").is_some_and(|value| value.trim() == "Flagged"),
        has_attachments: text(block, "HasAttachments")
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("true")),
        size: text(block, "Size").and_then(|value| value.trim().parse().ok()),
        message_id: text(block, "InternetMessageId"),
        in_reply_to: text(block, "InReplyTo"),
        references: text(block, "References"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RESPONSE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body>
    <m:FindFolderResponse xmlns:m="http://schemas.microsoft.com/exchange/services/2006/messages"
        xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types">
      <m:ResponseMessages>
        <m:FindFolderResponseMessage ResponseClass="Success">
          <m:ResponseCode>NoError</m:ResponseCode>
          <m:RootFolder TotalItemsInView="3" IncludesLastItemInRange="true">
            <t:Folders>
              <t:Folder>
                <t:FolderId Id="AAMkAD==" ChangeKey="CQAAABY"/>
                <t:DisplayName>收件箱</t:DisplayName>
                <t:TotalCount>42</t:TotalCount>
              </t:Folder>
              <t:Folder>
                <t:FolderId Id="AAMkAE=="/>
                <t:DisplayName>Projects &amp; Notes</t:DisplayName>
                <t:TotalCount>0</t:TotalCount>
              </t:Folder>
              <t:CalendarFolder>
                <t:FolderId Id="AAMkAF=="/>
                <t:DisplayName>Calendar</t:DisplayName>
              </t:CalendarFolder>
            </t:Folders>
          </m:RootFolder>
        </m:FindFolderResponseMessage>
      </m:ResponseMessages>
    </m:FindFolderResponse>
  </s:Body>
</s:Envelope>"#;

    #[test]
    fn a_folder_list_is_read_with_ids_names_and_counts() {
        let folders = parse_folders(RESPONSE).unwrap();
        assert_eq!(folders.len(), 3);
        assert_eq!(folders[0].id, "AAMkAD==");
        assert_eq!(folders[0].display_name, "收件箱");
        assert_eq!(folders[0].total_count, 42);
        assert_eq!(folders[1].display_name, "Projects & Notes");
        assert_eq!(folders[2].id, "AAMkAF==");
        assert_eq!(folders[2].total_count, 0);
    }

    #[test]
    fn a_server_side_failure_is_reported_without_its_message() {
        let xml = RESPONSE
            .replace(r#"ResponseClass="Success""#, r#"ResponseClass="Error""#)
            .replace("NoError", "ErrorAccessDenied");
        assert_eq!(parse_folders(&xml), Err(SoapError::Server));
    }

    #[test]
    fn a_dtd_or_a_foreign_body_is_refused() {
        assert_eq!(
            parse_folders(r#"<!DOCTYPE x [<!ENTITY e SYSTEM "file:///etc/passwd">]><Envelope/>"#),
            Err(SoapError::Malformed)
        );
        assert_eq!(parse_folders("<html>hi</html>"), Err(SoapError::Malformed));
        assert_eq!(parse_folders(""), Err(SoapError::Malformed));
    }

    #[test]
    fn prefixes_do_not_matter() {
        let plain = RESPONSE
            .replace("t:", "")
            .replace("m:", "")
            .replace("s:", "");
        let folders = parse_folders(&plain).unwrap();
        assert_eq!(folders.len(), 3);
        assert_eq!(folders[0].display_name, "收件箱");
    }

    #[test]
    fn the_request_names_the_version_and_the_root_folder() {
        let request = find_folders();
        assert!(request.contains("RequestServerVersion Version=\"Exchange2013\""));
        assert!(request.contains(r#"<t:DistinguishedFolderId Id="msgfolderroot"/>"#));
        assert!(request.contains("MaxEntriesReturned=\"256\""));
        assert!(request.ends_with("</soap:Envelope>"));
    }

    #[test]
    fn folder_pages_ask_for_the_parent_and_can_fall_back() {
        // The Type 3 probe stays on the bare shape: a rejected extra property
        // there would break sign-in before any retry could happen.
        assert_eq!(find_folders(), find_folders_plain(0));
        assert!(!find_folders().contains("FieldURI"));
        let page = find_folders_page(256);
        assert!(page.contains(r#"Offset="256" BasePoint="Beginning""#));
        assert!(page.contains(r#"Traversal="Deep""#));
        // `BaseShape=Default` does NOT include ParentFolderId / FolderClass.
        // Without these the deep listing can never be rebuilt into a tree.
        assert!(page.contains(r#"<t:FieldURI FieldURI="folder:ParentFolderId"/>"#));
        assert!(page.contains(r#"<t:FieldURI FieldURI="folder:FolderClass"/>"#));
        assert!(page.contains(r#"<t:FieldURI FieldURI="folder:UnreadCount"/>"#));
        assert!(page.contains(r#"<t:BaseShape>Default</t:BaseShape>"#));
        // The middle fallback needs no FieldURI at all to still get parents.
        let all = find_folders_all(256);
        assert!(all.contains(r#"<t:BaseShape>AllProperties</t:BaseShape>"#));
        assert!(!all.contains("FieldURI"));
        let plain = find_folders_plain(256);
        assert!(plain.contains(r#"Offset="256""#));
        assert!(!plain.contains("FieldURI"));
        assert!(plain.ends_with("</soap:Envelope>"));
    }

    #[test]
    fn shallow_children_requests_identify_their_parent() {
        let request = find_folder_children("A&B", 0);
        assert!(request.contains(r#"Traversal="Shallow""#));
        assert!(request.contains(r#"<t:FolderId Id="A&amp;B"/>"#));
        assert!(!request.contains("DistinguishedFolderId"));
        // A bare Default shape: no shape rejection can break the repair path.
        assert!(!request.contains("FieldURI"));
    }

    #[test]
    fn distinguished_folder_request_asks_for_ids_in_a_stable_order() {
        let request = get_distinguished_folders(&["inbox", "sentitems", "junkemail"]);
        assert!(request.contains(r#"<t:BaseShape>IdOnly</t:BaseShape>"#));
        assert!(request.contains(r#"<t:DistinguishedFolderId Id="inbox"/>"#));
        assert!(request.contains(r#"<t:DistinguishedFolderId Id="sentitems"/>"#));
        assert!(request.contains(r#"<t:DistinguishedFolderId Id="junkemail"/>"#));
        assert!(
            request.find(r#"Id="inbox""#).unwrap() < request.find(r#"Id="sentitems""#).unwrap()
        );
        assert!(request.ends_with("</soap:Envelope>"));
    }

    #[test]
    fn a_failed_getfolder_message_keeps_its_slot_empty() {
        let response = r#"<?xml version="1.0"?><s:Envelope><s:Body>
          <m:GetFolderResponse>
            <m:ResponseMessages>
              <m:GetFolderResponseMessage ResponseClass="Success">
                <m:Folders><t:Folder><t:FolderId Id="id-inbox" ChangeKey="ck"/></t:Folder></m:Folders>
              </m:GetFolderResponseMessage>
              <m:GetFolderResponseMessage ResponseClass="Error">
                <m:ResponseCode>ErrorFolderNotFound</m:ResponseCode>
              </m:GetFolderResponseMessage>
              <m:GetFolderResponseMessage ResponseClass="Success">
                <m:Folders><t:Folder><t:FolderId Id="id-sent"/></t:Folder></m:Folders>
              </m:GetFolderResponseMessage>
            </m:ResponseMessages>
          </m:GetFolderResponse>
        </s:Body></s:Envelope>"#;
        assert_eq!(
            parse_distinguished_folders(response),
            vec![
                Some("id-inbox".to_owned()),
                None,
                Some("id-sent".to_owned())
            ]
        );
        assert!(parse_distinguished_folders("<html/>").is_empty());
    }

    #[test]
    fn every_folder_field_uri_is_a_legal_ews_member() {
        for field in FOLDER_FIELDS {
            assert!(
                ALLOWED_FIELD_URIS.contains(field),
                "{field} is not a legal UnindexedFieldURIType value"
            );
        }
    }

    const PAGED_RESPONSE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body>
    <m:FindFolderResponse xmlns:m="http://schemas.microsoft.com/exchange/services/2006/messages"
        xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types">
      <m:ResponseMessages>
        <m:FindFolderResponseMessage ResponseClass="Success">
          <m:ResponseCode>NoError</m:ResponseCode>
          <m:RootFolder IndexedPagingOffset="256" TotalItemsInView="259" IncludesLastItemInRange="false">
            <t:Folders>
              <t:Folder>
                <t:FolderId Id="root" ChangeKey="ck"/>
                <t:ParentFolderId Id="msgroot" ChangeKey="ck"/>
                <t:FolderClass>IPF.Note</t:FolderClass>
                <t:DisplayName>收件箱</t:DisplayName>
                <t:TotalCount>12</t:TotalCount>
                <t:UnreadCount>3</t:UnreadCount>
                <t:ChildFolderCount>2</t:ChildFolderCount>
              </t:Folder>
              <t:CalendarFolder>
                <t:FolderId Id="cal"/>
                <t:ParentFolderId Id="msgroot"/>
                <t:FolderClass>IPF.Appointment</t:FolderClass>
                <t:DisplayName>日历</t:DisplayName>
              </t:CalendarFolder>
              <t:Folder>
                <t:FolderId Id="child"/>
                <t:ParentFolderId Id="root"/>
                <t:FolderClass>IPF.Note</t:FolderClass>
                <t:DisplayName>项目</t:DisplayName>
                <t:TotalCount>7</t:TotalCount>
                <t:UnreadCount>0</t:UnreadCount>
              </t:Folder>
            </t:Folders>
          </m:RootFolder>
        </m:FindFolderResponseMessage>
      </m:ResponseMessages>
    </m:FindFolderResponse>
  </s:Body>
</s:Envelope>"#;

    #[test]
    fn a_deep_listing_keeps_parents_classes_counts_and_document_order() {
        let page = parse_folders_page(PAGED_RESPONSE, 0).unwrap();
        assert_eq!(page.next_offset, Some(256));
        assert_eq!(page.folders.len(), 3);
        // A CalendarFolder sits between two Folders: grouping by element name
        // would have moved it to the end.
        assert_eq!(page.folders[1].id, "cal");
        let inbox = &page.folders[0];
        assert_eq!(inbox.parent_id.as_deref(), Some("msgroot"));
        assert_eq!(inbox.folder_class.as_deref(), Some("IPF.Note"));
        assert_eq!(inbox.unread_count, 3);
        assert_eq!(inbox.child_folder_count, Some(2));
        assert_eq!(page.folders[2].child_folder_count, None);
        assert!(inbox.mail_folder);
        assert!(!page.folders[1].mail_folder);
        assert_eq!(
            page.folders[1].folder_class.as_deref(),
            Some("IPF.Appointment")
        );
        assert_eq!(page.folders[2].parent_id.as_deref(), Some("root"));
        assert_eq!(page.folders[2].unread_count, 0);
    }

    const EMPTY_PAGE: &str = concat!(
        r#"<?xml version="1.0"?><s:Envelope><s:Body>"#,
        r#"<m:FindFolderResponse><m:ResponseMessages>"#,
        r#"<m:FindFolderResponseMessage ResponseClass="Success">"#,
        r#"<m:ResponseCode>NoError</m:ResponseCode>"#,
        r#"<m:RootFolder IndexedPagingOffset="256" IncludesLastItemInRange="false">"#,
        r#"<t:Folders></t:Folders></m:RootFolder>"#,
        r#"</m:FindFolderResponseMessage></m:ResponseMessages>"#,
        r#"</m:FindFolderResponse></s:Body></s:Envelope>"#,
    );

    #[test]
    fn paging_stops_on_the_last_page_or_when_it_cannot_advance() {
        let last = PAGED_RESPONSE.replace(
            r#"IncludesLastItemInRange="false""#,
            r#"IncludesLastItemInRange="true""#,
        );
        assert_eq!(parse_folders_page(&last, 0).unwrap().next_offset, None);
        // Without a server-supplied cursor, advance by the number of items read.
        let no_cursor = PAGED_RESPONSE.replace(r#"IndexedPagingOffset="256" "#, "");
        assert_eq!(
            parse_folders_page(&no_cursor, 0).unwrap().next_offset,
            Some(3)
        );
        // An empty page must end the loop even when the server offers a cursor.
        assert_eq!(parse_folders_page(EMPTY_PAGE, 0).unwrap().next_offset, None);
    }

    #[test]
    fn markup_in_a_folder_name_cannot_break_the_reader() {
        let hostile = RESPONSE.replace("收件箱", "&lt;script&gt;alert(1)&lt;/script&gt;");
        let folders = parse_folders(&hostile).unwrap();
        assert_eq!(folders[0].display_name, "<script>alert(1)</script>");
    }

    const ITEMS_RESPONSE: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/">
  <s:Body>
    <m:FindItemResponse xmlns:m="http://schemas.microsoft.com/exchange/services/2006/messages"
        xmlns:t="http://schemas.microsoft.com/exchange/services/2006/types">
      <m:ResponseMessages>
        <m:FindItemResponseMessage ResponseClass="Success">
          <m:ResponseCode>NoError</m:ResponseCode>
          <m:RootFolder TotalItemsInView="1" IncludesLastItemInRange="true">
            <t:Items>
              <t:Message>
                <t:ItemId Id="AAMkAGI2==" ChangeKey="CQAAABYAAA"/>
                <t:Subject>Quarterly &amp; notes</t:Subject>
                <t:From>
                  <t:Mailbox>
                    <t:Name>Alice Example</t:Name>
                    <t:EmailAddress>alice@example.com</t:EmailAddress>
                  </t:Mailbox>
                </t:From>
                <t:ToRecipients>
                  <t:Mailbox>
                    <t:Name>Bob</t:Name>
                    <t:EmailAddress>bob@example.com</t:EmailAddress>
                  </t:Mailbox>
                  <t:Mailbox>
                    <t:EmailAddress>carol@contoso.com</t:EmailAddress>
                  </t:Mailbox>
                </t:ToRecipients>
                <t:CcRecipients>
                  <t:Mailbox><t:EmailAddress>dave@example.com</t:EmailAddress></t:Mailbox>
                </t:CcRecipients>
                <t:ReplyTo>
                  <t:Mailbox><t:EmailAddress>reply@example.com</t:EmailAddress></t:Mailbox>
                </t:ReplyTo>
                <t:DateTimeSent>2026-01-02T03:04:05Z</t:DateTimeSent>
                <t:DateTimeReceived>2026-01-02T03:04:06Z</t:DateTimeReceived>
                <t:IsRead>false</t:IsRead>
                <t:HasAttachments>true</t:HasAttachments>
                <t:Size>4096</t:Size>
                <t:InternetMessageId>&lt;mid-1@example.com&gt;</t:InternetMessageId>
                <t:InReplyTo>&lt;mid-0@example.com&gt;</t:InReplyTo>
                <t:References>&lt;mid-a@example.com&gt; &lt;mid-b@example.com&gt;</t:References>
                <t:Flag><t:FlagStatus>Flagged</t:FlagStatus></t:Flag>
              </t:Message>
            </t:Items>
          </m:RootFolder>
        </m:FindItemResponseMessage>
      </m:ResponseMessages>
    </m:FindItemResponse>
  </s:Body>
</s:Envelope>"#;

    fn mime_response(body: &str) -> String {
        format!(
            concat!(
                r#"<?xml version="1.0"?>"#,
                r#"<s:Envelope><s:Body>"#,
                r#"<m:GetItemResponse><m:ResponseMessages>"#,
                r#"<m:GetItemResponseMessage ResponseClass="Success">"#,
                r#"<m:ResponseCode>NoError</m:ResponseCode>"#,
                r#"<m:Items><t:Message><t:ItemId Id="id-1"/>"#,
                r#"<t:MimeContent CharacterSet="UTF-8">{body}</t:MimeContent>"#,
                r#"</t:Message></m:Items>"#,
                r#"</m:GetItemResponseMessage></m:ResponseMessages>"#,
                r#"</m:GetItemResponse></s:Body></s:Envelope>"#,
            ),
            body = body,
        )
    }

    fn action_response(class: &str, code: &str) -> String {
        format!(
            concat!(
                r#"<?xml version="1.0"?>"#,
                r#"<s:Envelope><s:Body>"#,
                r#"<m:UpdateItemResponse><m:ResponseMessages>"#,
                r#"<m:UpdateItemResponseMessage ResponseClass="{class}">"#,
                r#"<m:ResponseCode>{code}</m:ResponseCode>"#,
                r#"</m:UpdateItemResponseMessage></m:ResponseMessages>"#,
                r#"</m:UpdateItemResponse></s:Body></s:Envelope>"#,
            ),
            class = class,
            code = code,
        )
    }

    #[test]
    fn a_full_find_item_response_yields_every_field() {
        let items = parse_items(ITEMS_RESPONSE).unwrap();
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.id, "AAMkAGI2==");
        assert_eq!(item.change_key.as_deref(), Some("CQAAABYAAA"));
        assert_eq!(item.subject, "Quarterly & notes");
        assert_eq!(
            item.from,
            Some(Party {
                name: Some("Alice Example".to_owned()),
                address: "alice@example.com".to_owned(),
            })
        );
        assert_eq!(item.to.len(), 2);
        assert_eq!(item.to[0].name.as_deref(), Some("Bob"));
        assert_eq!(item.to[0].address, "bob@example.com");
        assert_eq!(item.to[1].name, None);
        assert_eq!(item.to[1].address, "carol@contoso.com");
        assert_eq!(item.cc.len(), 1);
        assert_eq!(item.cc[0].address, "dave@example.com");
        assert_eq!(item.reply_to.len(), 1);
        assert_eq!(item.reply_to[0].address, "reply@example.com");
        assert_eq!(item.sent_at.as_deref(), Some("2026-01-02T03:04:05Z"));
        assert_eq!(item.received_at.as_deref(), Some("2026-01-02T03:04:06Z"));
        assert!(!item.is_read);
        assert!(item.is_flagged);
        assert!(item.has_attachments);
        assert_eq!(item.size, Some(4096));
        assert_eq!(item.message_id.as_deref(), Some("<mid-1@example.com>"));
        assert_eq!(item.in_reply_to.as_deref(), Some("<mid-0@example.com>"));
        assert_eq!(
            item.references.as_deref(),
            Some("<mid-a@example.com> <mid-b@example.com>")
        );
    }

    #[test]
    fn an_error_class_is_reported_as_a_server_failure() {
        let xml = ITEMS_RESPONSE.replace(r#"ResponseClass="Success""#, r#"ResponseClass="Error""#);
        assert_eq!(parse_items(&xml), Err(SoapError::Server));
    }

    #[test]
    fn a_dtd_or_a_foreign_body_is_refused_by_every_reader() {
        let xml = r#"<!DOCTYPE x [<!ENTITY e "x">]><Envelope/>"#;
        assert_eq!(parse_items(xml), Err(SoapError::Malformed));
        assert_eq!(parse_mime(xml), Err(SoapError::Malformed));
        assert_eq!(
            response_ok(xml, "AnyResponseMessage"),
            Err(SoapError::Malformed)
        );
        assert_eq!(parse_items("<html>hi</html>"), Err(SoapError::Malformed));
        assert_eq!(parse_mime("<html>hi</html>"), Err(SoapError::Malformed));
    }

    #[test]
    fn prefixes_and_self_closing_ids_are_accepted() {
        let plain = ITEMS_RESPONSE
            .replace("m:", "")
            .replace("t:", "")
            .replace("s:", "");
        let items = parse_items(&plain).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "AAMkAGI2==");
        assert_eq!(items[0].to.len(), 2);
        assert!(items[0].is_flagged);
    }

    #[test]
    fn unprefixed_item_elements_are_read_too() {
        let xml = concat!(
            r#"<?xml version="1.0"?><Envelope><Body><FindItemResponse>"#,
            r#"<ResponseMessages><FindItemResponseMessage ResponseClass="Success">"#,
            r#"<ResponseCode>NoError</ResponseCode><RootFolder><Items><Message>"#,
            r#"<ItemId Id="raw-id"/><Subject>Raw</Subject><IsRead>true</IsRead>"#,
            r#"<HasAttachments>false</HasAttachments><Size>7</Size>"#,
            r#"<InternetMessageId>raw-mid</InternetMessageId>"#,
            r#"</Message></Items></RootFolder></FindItemResponseMessage>"#,
            r#"</ResponseMessages></FindItemResponse></Body></Envelope>"#,
        );
        let items = parse_items(xml).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, "raw-id");
        assert_eq!(items[0].subject, "Raw");
        assert!(items[0].is_read);
        assert!(!items[0].has_attachments);
        assert_eq!(items[0].size, Some(7));
        assert_eq!(items[0].message_id.as_deref(), Some("raw-mid"));
    }

    #[test]
    fn missing_and_false_fields_take_defaults() {
        let xml = concat!(
            r#"<?xml version="1.0"?><s:Envelope><s:Body>"#,
            r#"<m:FindItemResponse><m:ResponseMessages>"#,
            r#"<m:FindItemResponseMessage ResponseClass="Success">"#,
            r#"<m:ResponseCode>NoError</m:ResponseCode>"#,
            r#"<m:RootFolder><t:Items><t:Message>"#,
            r#"<t:ItemId Id="only-id"/>"#,
            r#"<t:Flag><t:FlagStatus>NotFlagged</t:FlagStatus></t:Flag>"#,
            r#"</t:Message></t:Items></m:RootFolder>"#,
            r#"</m:FindItemResponseMessage></m:ResponseMessages>"#,
            r#"</m:FindItemResponse></s:Body></s:Envelope>"#,
        );
        let items = parse_items(xml).unwrap();
        let item = &items[0];
        assert_eq!(item.id, "only-id");
        assert_eq!(item.change_key, None);
        assert_eq!(item.subject, "");
        assert_eq!(item.from, None);
        assert!(item.to.is_empty());
        assert!(item.cc.is_empty());
        assert!(item.reply_to.is_empty());
        assert_eq!(item.sent_at, None);
        assert_eq!(item.received_at, None);
        assert!(!item.is_read);
        assert!(!item.is_flagged);
        assert!(!item.has_attachments);
        assert_eq!(item.size, None);
        assert_eq!(item.message_id, None);
        assert_eq!(item.in_reply_to, None);
        assert_eq!(item.references, None);
    }

    #[test]
    fn entities_in_item_text_are_decoded() {
        let xml = ITEMS_RESPONSE.replace(
            "Quarterly &amp; notes",
            "A &amp; B &lt;x&gt; &quot;q&quot; &apos;s",
        );
        let items = parse_items(&xml).unwrap();
        assert_eq!(items[0].subject, "A & B <x> \"q\" 's");
    }

    #[test]
    fn a_folder_id_is_escaped_before_it_reaches_the_request() {
        let request = find_items("a&b\"c<d>e'f", 10);
        assert!(request.contains(r#"Traversal="Shallow""#));
        assert!(request.contains("a&amp;b&quot;c&lt;d&gt;e&apos;f"));
        assert!(!request.contains("a&b"));
    }

    #[test]
    fn max_entries_is_rendered_as_a_plain_number() {
        let request = find_items("inbox", 37);
        assert!(request.contains(r#"MaxEntriesReturned="37""#));
        assert!(request.contains(r#"Offset="0" BasePoint="Beginning""#));

        let huge = find_items("inbox", usize::MAX);
        let marker = "MaxEntriesReturned=\"";
        let start = huge.find(marker).unwrap() + marker.len();
        let tail = &huge[start..];
        let digits = &tail[..tail.find('"').unwrap()];
        assert!(digits.chars().all(|character| character.is_ascii_digit()));
        assert_eq!(digits, usize::MAX.to_string());
    }

    #[test]
    fn find_item_asks_for_every_field_the_item_needs() {
        let request = find_items("inbox", 5);
        for field in [
            "item:Subject",
            "message:From",
            "message:ToRecipients",
            "message:CcRecipients",
            "message:ReplyTo",
            "item:DateTimeSent",
            "item:DateTimeReceived",
            "message:IsRead",
            "item:HasAttachments",
            "item:Size",
            "message:InternetMessageId",
            "item:InReplyTo",
            "message:References",
            "item:Flag",
        ] {
            assert!(
                request.contains(&format!("FieldURI=\"{field}\"")),
                "missing {field}"
            );
        }
        // `message:InReplyTo` is not a member of UnindexedFieldURIType: sending
        // it makes Exchange fail XML deserialization with ErrorInvalidRequest
        // (HTTP 500) instead of returning any items at all.
        assert!(!request.contains("message:InReplyTo"));
        assert!(request.contains(r#"<t:BaseShape>IdOnly</t:BaseShape>"#));
        assert!(request.contains("RequestServerVersion Version=\"Exchange2013\""));
        assert!(request.ends_with("</soap:Envelope>"));
    }

    #[test]
    fn every_emitted_field_uri_is_a_legal_ews_member() {
        for field in ITEM_FIELDS.iter().chain(BASIC_ITEM_FIELDS) {
            assert!(
                ALLOWED_FIELD_URIS.contains(field),
                "{field} is not a legal UnindexedFieldURIType value"
            );
        }
        // The enumeration has no `message:InReplyTo`; `InReplyTo` is an item
        // property (itemInReplyTo) even though it is an RFC 5322 header.
        assert!(!ALLOWED_FIELD_URIS.contains(&"message:InReplyTo"));
        for request in [
            find_items("inbox", 5),
            update_flags("id", None, Some(true), Some(true)),
        ] {
            assert!(!request.contains("message:InReplyTo"));
        }
    }

    #[test]
    fn the_basic_find_item_set_drops_only_the_late_properties() {
        let full = find_items("inbox", 5);
        let basic = find_items_basic("inbox", 5);
        assert!(basic.contains(r#"<m:FindItem Traversal="Shallow">"#));
        assert!(basic.contains(r#"MaxEntriesReturned="5""#));
        assert!(basic.ends_with("</soap:Envelope>"));
        assert!(basic.matches("<t:FieldURI ").count() == BASIC_ITEM_FIELDS.len());
        assert!(full.matches("<t:FieldURI ").count() == ITEM_FIELDS.len());
        for late in [
            "message:ReplyTo",
            "item:InReplyTo",
            "message:References",
            "item:Flag",
        ] {
            assert!(full.contains(&format!("FieldURI=\"{late}\"")));
            assert!(!basic.contains(&format!("FieldURI=\"{late}\"")));
        }
        // Folder ids stay escaped in the reduced request too.
        let escaped = find_items_basic("a&b\"c", 1);
        assert!(escaped.contains("a&amp;b&quot;c"));
    }

    #[test]
    fn get_item_asks_for_mime_content() {
        let request = get_item_mime("AAMkAGI2==", Some("CQAAABYAAA"));
        assert!(request.contains(r#"<m:GetItem>"#));
        assert!(request.contains(r#"<t:IncludeMimeContent>true</t:IncludeMimeContent>"#));
        assert!(request.contains(r#"<t:BaseShape>IdOnly</t:BaseShape>"#));
        assert!(request.contains(r#"<t:ItemId Id="AAMkAGI2==" ChangeKey="CQAAABYAAA"/>"#));
        assert!(request.contains(r#"<m:ItemIds>"#));
    }

    #[test]
    fn the_change_key_is_optional_on_item_references() {
        let with = get_item_mime("id-1", Some("ck-1"));
        assert!(with.contains(r#"ChangeKey="ck-1""#));
        let without = get_item_mime("id-1", None);
        assert!(!without.contains("ChangeKey"));
        assert!(without.contains(r#"<t:ItemId Id="id-1"/>"#));

        let delete_with = delete_item("id-2", Some("ck-2"));
        assert!(delete_with.contains(r#"ChangeKey="ck-2""#));
        let delete_without = delete_item("id-2", None);
        assert!(!delete_without.contains("ChangeKey"));
    }

    #[test]
    fn item_ids_are_escaped_in_every_request() {
        let hostile = "id&\"<>'";
        for request in [
            get_item_mime(hostile, None),
            update_flags(hostile, None, Some(true), None),
            delete_item(hostile, None),
        ] {
            assert!(request.contains("id&amp;&quot;&lt;&gt;&apos;"), "{request}");
            assert!(!request.contains("id&\""), "{request}");
        }
    }

    #[test]
    fn update_item_builds_one_set_field_per_flag() {
        let both = update_flags("id-1", Some("ck-1"), Some(true), Some(false));
        assert!(both.contains(r#"MessageDisposition="SaveOnly" ConflictResolution="AutoResolve""#));
        assert!(both.contains(r#"<t:FieldURI FieldURI="message:IsRead"/>"#));
        assert!(both.contains(r#"<t:IsRead>true</t:IsRead>"#));
        assert!(both.contains(r#"<t:FieldURI FieldURI="item:Flag"/>"#));
        assert!(both.contains(r#"<t:FlagStatus>NotFlagged</t:FlagStatus>"#));
        assert_eq!(both.matches("<t:SetItemField>").count(), 2);

        let read_only = update_flags("id-1", None, Some(false), None);
        assert_eq!(read_only.matches("<t:SetItemField>").count(), 1);
        assert!(read_only.contains(r#"<t:IsRead>false</t:IsRead>"#));
        assert!(!read_only.contains("FlagStatus"));

        let flagged_only = update_flags("id-1", None, None, Some(true));
        assert!(flagged_only.contains(r#"<t:FlagStatus>Flagged</t:FlagStatus>"#));
        assert_eq!(flagged_only.matches("<t:SetItemField>").count(), 1);
    }

    #[test]
    fn update_item_with_no_flags_is_still_well_formed() {
        let request = update_flags("id-1", None, None, None);
        assert!(request.contains("<m:UpdateItem"));
        assert!(request.contains("<t:Updates></t:Updates>"));
        assert!(!request.contains("<t:SetItemField>"));
        assert!(request.ends_with("</soap:Envelope>"));
    }

    #[test]
    fn delete_item_moves_to_deleted_items_without_cancellations() {
        let request = delete_item("id-1", Some("ck-1"));
        assert!(request.contains(
            r#"<m:DeleteItem DeleteType="MoveToDeletedItems" SendMeetingCancellations="None">"#
        ));
        assert!(
            request.contains(r#"<m:ItemIds><t:ItemId Id="id-1" ChangeKey="ck-1"/></m:ItemIds>"#)
        );
    }

    #[test]
    fn create_item_sends_and_keeps_a_copy() {
        let request = create_and_send(b"hello");
        assert!(request.contains(r#"<m:CreateItem MessageDisposition="SendAndSaveCopy">"#));
        assert!(request.contains(r#"<t:DistinguishedFolderId Id="sentitems"/>"#));
        assert!(
            request.contains(r#"<t:MimeContent CharacterSet="UTF-8">aGVsbG8=</t:MimeContent>"#)
        );
        assert!(request.contains("<m:Items><t:Message>"));
        assert!(request.ends_with("</soap:Envelope>"));
    }

    #[test]
    fn mime_content_is_decoded_and_whitespace_is_ignored() {
        let xml = mime_response("SGVsbG8s\nIHdvcmxk\nIQ==\n");
        assert_eq!(parse_mime(&xml).unwrap(), b"Hello, world!".to_vec());
    }

    #[test]
    fn mime_reads_report_an_error_class_as_a_server_failure() {
        let xml = mime_response("aGVsbG8=")
            .replace(r#"ResponseClass="Success""#, r#"ResponseClass="Error""#);
        assert_eq!(parse_mime(&xml), Err(SoapError::Server));
    }

    #[test]
    fn an_empty_or_self_closing_mime_content_is_refused() {
        assert_eq!(parse_mime(&mime_response("")), Err(SoapError::Malformed));
        let xml = mime_response("").replace(
            r#"<t:MimeContent CharacterSet="UTF-8"></t:MimeContent>"#,
            r#"<t:MimeContent CharacterSet="UTF-8"/>"#,
        );
        assert_eq!(parse_mime(&xml), Err(SoapError::Malformed));
    }

    #[test]
    fn an_oversized_mime_payload_is_refused_without_decoding() {
        let oversized = "A".repeat(MAX_MIME_BYTES + 1);
        let xml = mime_response(&oversized);
        assert_eq!(parse_mime(&xml), Err(SoapError::Malformed));
    }

    #[test]
    fn response_ok_accepts_a_success_with_no_error() {
        assert_eq!(
            response_ok(
                &action_response("Success", "NoError"),
                "UpdateItemResponseMessage"
            ),
            Ok(())
        );
    }

    #[test]
    fn response_ok_accepts_a_bare_response_message() {
        let fragment = concat!(
            r#"<m:UpdateItemResponseMessage ResponseClass="Success">"#,
            r#"<m:ResponseCode>NoError</m:ResponseCode>"#,
            r#"</m:UpdateItemResponseMessage>"#,
        );
        assert_eq!(response_ok(fragment, "UpdateItemResponseMessage"), Ok(()));
    }

    #[test]
    fn response_ok_rejects_an_error_class_or_a_failure_code() {
        assert_eq!(
            response_ok(
                &action_response("Error", "NoError"),
                "UpdateItemResponseMessage"
            ),
            Err(SoapError::Server)
        );
        assert_eq!(
            response_ok(
                &action_response("Success", "ErrorAccessDenied"),
                "UpdateItemResponseMessage"
            ),
            Err(SoapError::Server)
        );
    }

    #[test]
    fn response_ok_needs_the_named_response_message() {
        assert_eq!(
            response_ok(
                &action_response("Success", "NoError"),
                "DeleteItemResponseMessage"
            ),
            Err(SoapError::Malformed)
        );
    }
}
