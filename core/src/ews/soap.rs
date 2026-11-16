//! Bounded, namespace-aware EWS SOAP construction and parsing. DTD/entity
//! declarations are disabled in roxmltree; element names alone never confer trust.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use roxmltree::{Document, Node, ParsingOptions};

use super::{EwsError, EwsFolder, EwsItem, EwsPage};
use crate::mail::MailAddress;
use crate::smtp::message::SmtpEnvelope;

pub(super) const MESSAGES: &str = "http://schemas.microsoft.com/exchange/services/2006/messages";
pub(super) const TYPES: &str = "http://schemas.microsoft.com/exchange/services/2006/types";
const SOAP: &str = "http://schemas.xmlsoap.org/soap/envelope/";

pub(super) fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn envelope(body: String) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><s:Envelope xmlns:s=\"{SOAP}\" xmlns:m=\"{MESSAGES}\" xmlns:t=\"{TYPES}\"><s:Header><t:RequestServerVersion Version=\"Exchange2013_SP1\"/></s:Header><s:Body>{body}</s:Body></s:Envelope>"
    )
}

pub(super) fn probe() -> String {
    distinguished(&["inbox"])
}

/// One `GetFolder` for several well-known folders; the server answers with
/// one response message per id, in request order.
pub(super) fn distinguished(names: &[&str]) -> String {
    envelope(format!(
        "<m:GetFolder><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape></m:FolderShape><m:FolderIds>{}</m:FolderIds></m:GetFolder>",
        names
            .iter()
            .map(|name| format!("<t:DistinguishedFolderId Id=\"{}\"/>", escape(name)))
            .collect::<String>()
    ))
}

pub(super) fn folders(offset: usize) -> String {
    envelope(format!(
        "<m:FindFolder Traversal=\"Deep\"><m:FolderShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{}</t:AdditionalProperties></m:FolderShape><m:IndexedPageFolderView MaxEntriesReturned=\"256\" Offset=\"{offset}\" BasePoint=\"Beginning\"/><m:ParentFolderIds><t:DistinguishedFolderId Id=\"msgfolderroot\"/></m:ParentFolderIds></m:FindFolder>",
        fields(&[
            "folder:DisplayName",
            "folder:ParentFolderId",
            "folder:FolderClass",
            "folder:TotalCount",
            "folder:UnreadCount"
        ])
    ))
}

fn fields(names: &[&str]) -> String {
    names
        .iter()
        .map(|name| format!("<t:FieldURI FieldURI=\"{name}\"/>"))
        .collect()
}

pub(super) fn items(id: &str, since: DateTime<Utc>, offset: usize, limit: usize) -> String {
    envelope(format!(
        concat!(
            "<m:FindItem Traversal=\"Shallow\"><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape>",
            "<t:AdditionalProperties>{properties}</t:AdditionalProperties></m:ItemShape>",
            "<m:IndexedPageItemView MaxEntriesReturned=\"{limit}\" Offset=\"{offset}\" BasePoint=\"Beginning\"/>",
            "<m:Restriction><t:IsGreaterThanOrEqualTo><t:FieldURI FieldURI=\"item:DateTimeReceived\"/>",
            "<t:FieldURIOrConstant><t:Constant Value=\"{since}\"/></t:FieldURIOrConstant></t:IsGreaterThanOrEqualTo></m:Restriction>",
            "<m:SortOrder><t:FieldOrder Order=\"Descending\"><t:FieldURI FieldURI=\"item:DateTimeReceived\"/></t:FieldOrder></m:SortOrder>",
            "<m:ParentFolderIds><t:FolderId Id=\"{id}\"/></m:ParentFolderIds></m:FindItem>"
        ),
        properties = fields(&[
            "item:Subject",
            "item:DateTimeReceived",
            "item:DateTimeSent",
            "item:Size",
            "item:HasAttachments",
            "item:Flag",
            "item:InReplyTo",
            "message:IsRead",
            "message:InternetMessageId",
            "message:From",
            "message:References"
        ]),
        since = since.to_rfc3339(),
        id = escape(id),
        limit = limit,
        offset = offset
    ))
}

fn item_id(id: &str, key: Option<&str>) -> String {
    let key = key
        .map(|key| format!(" ChangeKey=\"{}\"", escape(key)))
        .unwrap_or_default();
    format!("<t:ItemId Id=\"{}\"{key}/>", escape(id))
}

pub(super) fn get_mime(id: &str, key: Option<&str>) -> String {
    envelope(format!(
        "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:IncludeMimeContent>true</t:IncludeMimeContent></m:ItemShape><m:ItemIds>{}</m:ItemIds></m:GetItem>",
        item_id(id, key)
    ))
}

/// One `GetItem` for the text bodies of several items.
pub(super) fn get_previews(items: &[(&str, Option<&str>)]) -> String {
    envelope(format!(
        "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:BodyType>Text</t:BodyType><t:AdditionalProperties><t:FieldURI FieldURI=\"item:Body\"/></t:AdditionalProperties></m:ItemShape><m:ItemIds>{}</m:ItemIds></m:GetItem>",
        items
            .iter()
            .map(|(id, key)| item_id(id, *key))
            .collect::<String>()
    ))
}

pub(super) fn get_recipients(ids: &[&str]) -> String {
    envelope(format!(
        "<m:GetItem><m:ItemShape><t:BaseShape>IdOnly</t:BaseShape><t:AdditionalProperties>{}</t:AdditionalProperties></m:ItemShape><m:ItemIds>{}</m:ItemIds></m:GetItem>",
        fields(&[
            "message:ToRecipients",
            "message:CcRecipients",
            "message:ReplyTo"
        ]),
        ids.iter().map(|id| item_id(id, None)).collect::<String>()
    ))
}

pub(super) fn flags(id: &str, read: Option<bool>, flagged: Option<bool>) -> String {
    let mut updates = String::new();
    if let Some(read) = read {
        updates.push_str(&format!("<t:SetItemField><t:FieldURI FieldURI=\"message:IsRead\"/><t:Message><t:IsRead>{read}</t:IsRead></t:Message></t:SetItemField>"));
    }
    if let Some(flagged) = flagged {
        updates.push_str(&format!("<t:SetItemField><t:FieldURI FieldURI=\"item:Flag\"/><t:Message><t:Flag><t:FlagStatus>{}</t:FlagStatus></t:Flag></t:Message></t:SetItemField>", if flagged { "Flagged" } else { "NotFlagged" }));
    }
    envelope(format!(
        "<m:UpdateItem MessageDisposition=\"SaveOnly\" ConflictResolution=\"AlwaysOverwrite\"><m:ItemChanges><t:ItemChange>{}<t:Updates>{updates}</t:Updates></t:ItemChange></m:ItemChanges></m:UpdateItem>",
        item_id(id, None)
    ))
}

pub(super) fn delete(id: &str, permanent: bool) -> String {
    envelope(format!(
        "<m:DeleteItem DeleteType=\"{}\" SendMeetingCancellations=\"SendToNone\" AffectedTaskOccurrences=\"AllOccurrences\"><m:ItemIds>{}</m:ItemIds></m:DeleteItem>",
        if permanent {
            "HardDelete"
        } else {
            "MoveToDeletedItems"
        },
        item_id(id, None)
    ))
}

pub(super) fn send(mime: &[u8], envelope_to: &SmtpEnvelope) -> Result<String, EwsError> {
    if mime.is_empty() || mime.len() > 32 * 1024 * 1024 || envelope_to.recipients.is_empty() {
        return Err(EwsError::TooLarge);
    }
    let parsed = mail_parser::MessageParser::default()
        .parse(mime)
        .ok_or(EwsError::Protocol)?;
    // The frozen envelope is authoritative; headers determine visible roles,
    // never who receives a message. Bcc is deliberately absent from MIME.
    if parsed.bcc().is_some() {
        return Err(EwsError::Protocol);
    }
    let visible = |addresses: Option<&mail_parser::Address<'_>>| -> Vec<String> {
        addresses
            .into_iter()
            .flat_map(|value| value.iter())
            .filter_map(|value| value.address())
            .map(str::to_owned)
            .collect()
    };
    let to = visible(parsed.to());
    let cc = visible(parsed.cc());
    let mut unique = std::collections::HashSet::new();
    let mut to_final = Vec::new();
    let mut cc_final = Vec::new();
    let mut bcc_final = Vec::new();
    for address in &envelope_to.recipients {
        if !crate::drafts::validate::valid_address(address) {
            return Err(EwsError::Protocol);
        }
        if !unique.insert(address.to_ascii_lowercase()) {
            continue;
        }
        if to.iter().any(|value| value.eq_ignore_ascii_case(address)) {
            to_final.push(address.as_str());
        } else if cc.iter().any(|value| value.eq_ignore_ascii_case(address)) {
            cc_final.push(address.as_str());
        } else {
            bcc_final.push(address.as_str());
        }
    }
    if !crate::drafts::validate::valid_address(&envelope_to.from) {
        return Err(EwsError::Protocol);
    }
    let recipients = |name: &str, addresses: &[&str]| {
        format!(
            "<t:{name}>{}</t:{name}>",
            addresses
                .iter()
                .map(|address| format!(
                    "<t:Mailbox><t:EmailAddress>{}</t:EmailAddress></t:Mailbox>",
                    escape(address)
                ))
                .collect::<String>()
        )
    };
    Ok(envelope(format!(
        concat!(
            "<m:CreateItem MessageDisposition=\"SendAndSaveCopy\"><m:SavedItemFolderId><t:DistinguishedFolderId Id=\"sentitems\"/></m:SavedItemFolderId>",
            "<m:Items><t:Message><t:MimeContent>{mime}</t:MimeContent>",
            "{to}{cc}{bcc}<t:From><t:Mailbox><t:EmailAddress>{from}</t:EmailAddress></t:Mailbox></t:From>",
            "</t:Message></m:Items></m:CreateItem>"
        ),
        mime = STANDARD.encode(mime),
        from = escape(&envelope_to.from),
        to = recipients("ToRecipients", &to_final),
        cc = recipients("CcRecipients", &cc_final),
        bcc = recipients("BccRecipients", &bcc_final)
    )))
}

pub(super) fn document(bytes: &[u8]) -> Result<Document<'_>, EwsError> {
    if bytes.len() > super::http::MAX_RESPONSE {
        return Err(EwsError::TooLarge);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| EwsError::Protocol)?;
    Document::parse_with_options(
        text,
        ParsingOptions {
            allow_dtd: false,
            nodes_limit: 250_000,
            ..ParsingOptions::default()
        },
    )
    .map_err(|_| EwsError::Protocol)
}

fn child<'a, 'i>(node: Node<'a, 'i>, ns: &str, name: &str) -> Option<Node<'a, 'i>> {
    node.children().find(|child| child.has_tag_name((ns, name)))
}

fn response<'a>(document: &'a Document<'a>, name: &str) -> Result<Node<'a, 'a>, EwsError> {
    let messages = response_messages(document, name)?;
    let mut responses = messages.children().filter(|node| node.is_element());
    let response = responses.next().ok_or(EwsError::Protocol)?;
    if responses.next().is_some() {
        return Err(EwsError::Protocol);
    }
    check_response(response, name)?;
    Ok(response)
}

fn response_messages<'a>(document: &'a Document<'a>, name: &str) -> Result<Node<'a, 'a>, EwsError> {
    let root = document.root_element();
    if !root.has_tag_name((SOAP, "Envelope")) {
        return Err(EwsError::Protocol);
    }
    let body = child(root, SOAP, "Body").ok_or(EwsError::Protocol)?;
    let operation = name.strip_suffix("Message").ok_or(EwsError::Protocol)?;
    let response = child(body, MESSAGES, operation).ok_or(EwsError::Protocol)?;
    child(response, MESSAGES, "ResponseMessages").ok_or(EwsError::Protocol)
}

fn check_response(response: Node<'_, '_>, name: &str) -> Result<(), EwsError> {
    if !response.has_tag_name((MESSAGES, name)) {
        return Err(EwsError::Protocol);
    }
    let code = child(response, MESSAGES, "ResponseCode")
        .and_then(|node| node.text())
        .ok_or(EwsError::Protocol)?;
    if response.attribute("ResponseClass") != Some("Success") || code != "NoError" {
        return Err(server_error(code));
    }
    Ok(())
}

pub(super) struct Recipients {
    pub id: String,
    pub to: Vec<MailAddress>,
    pub cc: Vec<MailAddress>,
    pub reply_to: Vec<MailAddress>,
}

pub(super) fn parse_recipients(
    bytes: &[u8],
    expected: &[&str],
) -> Result<Vec<Recipients>, EwsError> {
    let doc = document(bytes)?;
    let messages = response_messages(&doc, "GetItemResponseMessage")?;
    let responses: Vec<_> = messages.children().filter(Node::is_element).collect();
    if responses.len() != expected.len() {
        return Err(EwsError::Protocol);
    }
    let mut result = Vec::new();
    for (response, expected_id) in responses.into_iter().zip(expected) {
        match check_response(response, "GetItemResponseMessage") {
            // An item can disappear between FindItem and this metadata fetch.
            // Preserve its search result until the next authoritative scan.
            Err(EwsError::NotFound) => continue,
            other => other?,
        }
        let items = child(response, MESSAGES, "Items").ok_or(EwsError::Protocol)?;
        let mut items = items.children().filter(Node::is_element);
        let item = items.next().ok_or(EwsError::Protocol)?;
        if items.next().is_some() {
            return Err(EwsError::Protocol);
        }
        let (id, _) = identifier(item, "ItemId")?;
        if id != *expected_id {
            return Err(EwsError::Protocol);
        }
        result.push(Recipients {
            id,
            to: mailboxes(item, "ToRecipients"),
            cc: mailboxes(item, "CcRecipients"),
            reply_to: mailboxes(item, "ReplyTo"),
        });
    }
    Ok(result)
}

fn server_error(code: &str) -> EwsError {
    match code {
        "ErrorItemNotFound" | "ErrorFolderNotFound" => EwsError::NotFound,
        "ErrorAccessDenied" | "ErrorInvalidClientAccessTokenRequest" | "ErrorInvalidUserInfo" => {
            EwsError::Authentication
        }
        "ErrorServerBusy"
        | "ErrorExceededConnectionCount"
        | "ErrorMailboxStoreUnavailable"
        | "ErrorChangeKeyRequired"
        | "ErrorInvalidChangeKey"
        | "ErrorIrresolvableConflict"
        | "ErrorInvalidPropertyRequest"
        | "ErrorInvalidPropertyForOperation"
        | "ErrorInvalidIdMalformed"
        | "ErrorMessageSizeExceeded"
        | "ErrorQuotaExceeded"
        | "ErrorInvalidRecipients"
        | "ErrorSendAsDenied"
        | "ErrorRecipientNotFound"
        | "ErrorOperationNotAllowedWithPublicFolderRoot"
        | "ErrorInvalidFolderTypeForOperation"
        | "ErrorInvalidRequest"
        | "ErrorNonExistentMailbox" => EwsError::Server {
            code: code.to_owned(),
        },
        _ => EwsError::Server {
            code: "ErrorUnknown".into(),
        },
    }
}

pub(super) fn check(bytes: &[u8], name: &str) -> Result<(), EwsError> {
    let doc = document(bytes)?;
    response(&doc, name).map(|_| ())
}

fn text(node: Node<'_, '_>, name: &str) -> Option<String> {
    child(node, TYPES, name)
        .and_then(|node| node.text())
        .map(|value| value.chars().take(16_384).collect())
}

fn identifier(node: Node<'_, '_>, name: &str) -> Result<(String, Option<String>), EwsError> {
    let node = child(node, TYPES, name).ok_or(EwsError::Protocol)?;
    let id = node
        .attribute("Id")
        .filter(|id| !id.is_empty() && id.len() <= 8192)
        .ok_or(EwsError::Protocol)?;
    let key = node
        .attribute("ChangeKey")
        .filter(|key| key.len() <= 8192)
        .map(str::to_owned);
    Ok((id.into(), key))
}

fn page_offset(root: Node<'_, '_>, offset: usize, count: usize) -> Result<Option<usize>, EwsError> {
    match root.attribute("IncludesLastItemInRange") {
        Some("true") | Some("1") => Ok(None),
        Some("false") | Some("0") => {
            let next = root
                .attribute("IndexedPagingOffset")
                .and_then(|value| value.parse::<usize>().ok())
                .ok_or(EwsError::Protocol)?;
            if count == 0 || next <= offset || next > 1_000_000 {
                return Err(EwsError::Protocol);
            }
            Ok(Some(next))
        }
        _ => Err(EwsError::Protocol),
    }
}

pub(super) fn parse_folders(
    bytes: &[u8],
    offset: usize,
) -> Result<(Vec<EwsFolder>, Option<usize>), EwsError> {
    let doc = document(bytes)?;
    let response = response(&doc, "FindFolderResponseMessage")?;
    let root = child(response, MESSAGES, "RootFolder").ok_or(EwsError::Protocol)?;
    let nodes = child(root, TYPES, "Folders").ok_or(EwsError::Protocol)?;
    let mut folders = Vec::new();
    let mut count = 0;
    for node in nodes.children().filter(Node::is_element) {
        if node.tag_name().namespace() != Some(TYPES) {
            return Err(EwsError::Protocol);
        }
        count += 1;
        if count > 256 {
            return Err(EwsError::TooLarge);
        }
        // Calendar/task/contact/search folders are not regular mail folders.
        if node.tag_name().name() != "Folder" {
            continue;
        }
        if text(node, "FolderClass").is_some_and(|class| !class.starts_with("IPF.Note")) {
            continue;
        }
        let (id, _) = identifier(node, "FolderId")?;
        folders.push(EwsFolder {
            id,
            parent_id: identifier(node, "ParentFolderId").ok().map(|value| value.0),
            name: text(node, "DisplayName").unwrap_or_default(),
            role: None,
            total: text(node, "TotalCount")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
            unread: text(node, "UnreadCount")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
        });
    }
    let next = page_offset(root, offset, count)?;
    Ok((folders, next))
}

/// One result per requested folder, in request order. A malformed or
/// mislabelled reply fails the whole call; a refused id fails only itself.
pub(super) fn folder_ids(
    bytes: &[u8],
    expected: usize,
) -> Result<Vec<Result<String, EwsError>>, EwsError> {
    let doc = document(bytes)?;
    let messages = response_messages(&doc, "GetFolderResponseMessage")?;
    let responses: Vec<_> = messages.children().filter(Node::is_element).collect();
    if responses.len() != expected {
        return Err(EwsError::Protocol);
    }
    let mut result = Vec::with_capacity(expected);
    for response in responses {
        if let Err(error) = check_response(response, "GetFolderResponseMessage") {
            if error == EwsError::Protocol {
                return Err(error);
            }
            result.push(Err(error));
            continue;
        }
        let folders = child(response, MESSAGES, "Folders").ok_or(EwsError::Protocol)?;
        let folder = folders
            .children()
            .find(Node::is_element)
            .ok_or(EwsError::Protocol)?;
        result.push(Ok(identifier(folder, "FolderId")?.0));
    }
    Ok(result)
}

fn mailboxes(node: Node<'_, '_>, name: &str) -> Vec<MailAddress> {
    child(node, TYPES, name)
        .into_iter()
        .flat_map(|node| node.children())
        .filter(|node| node.has_tag_name((TYPES, "Mailbox")))
        .filter_map(|node| {
            Some(MailAddress {
                name: text(node, "Name"),
                address: text(node, "EmailAddress")?,
            })
        })
        .collect()
}

pub(super) fn parse_items(bytes: &[u8], offset: usize) -> Result<EwsPage, EwsError> {
    let doc = document(bytes)?;
    let response = response(&doc, "FindItemResponseMessage")?;
    let root = child(response, MESSAGES, "RootFolder").ok_or(EwsError::Protocol)?;
    let nodes = child(root, TYPES, "Items").ok_or(EwsError::Protocol)?;
    let mut items = Vec::new();
    let mut count = 0;
    for node in nodes.children().filter(Node::is_element) {
        if node.tag_name().namespace() != Some(TYPES) {
            return Err(EwsError::Protocol);
        }
        count += 1;
        if count > 500 {
            return Err(EwsError::TooLarge);
        }
        // Meeting invitations also carry Message fields; unrelated items don't.
        if !matches!(
            node.tag_name().name(),
            "Message" | "MeetingRequest" | "MeetingCancellation" | "MeetingResponse"
        ) {
            continue;
        }
        let (id, change_key) = identifier(node, "ItemId")?;
        let date = |name| {
            text(node, name)
                .and_then(|value| DateTime::parse_from_rfc3339(&value).ok())
                .map(|value| value.with_timezone(&Utc))
        };
        items.push(EwsItem {
            id,
            change_key,
            internet_message_id: text(node, "InternetMessageId"),
            subject: text(node, "Subject").unwrap_or_default(),
            from: mailboxes(node, "From").into_iter().next(),
            to: mailboxes(node, "ToRecipients"),
            cc: mailboxes(node, "CcRecipients"),
            reply_to: mailboxes(node, "ReplyTo"),
            received_at: date("DateTimeReceived"),
            sent_at: date("DateTimeSent"),
            is_read: text(node, "IsRead").is_some_and(|value| value == "true" || value == "1"),
            is_flagged: child(node, TYPES, "Flag")
                .and_then(|flag| text(flag, "FlagStatus"))
                .is_some_and(|flag| flag == "Flagged" || flag == "Complete"),
            size: text(node, "Size")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
            has_attachments: text(node, "HasAttachments")
                .is_some_and(|value| value == "true" || value == "1"),
            preview: None,
            in_reply_to: text(node, "InReplyTo"),
            references: text(node, "References"),
        });
    }
    Ok(EwsPage {
        items,
        scanned: count,
        next_offset: page_offset(root, offset, count)?,
    })
}

fn get_item<'a>(doc: &'a Document<'a>) -> Result<Node<'a, 'a>, EwsError> {
    let response = response(doc, "GetItemResponseMessage")?;
    child(response, MESSAGES, "Items")
        .and_then(|items| items.children().find(Node::is_element))
        .ok_or(EwsError::Protocol)
}

pub(super) fn parse_mime(bytes: &[u8], limit: usize) -> Result<Vec<u8>, EwsError> {
    let doc = document(bytes)?;
    let item = get_item(&doc)?;
    let encoded = child(item, TYPES, "MimeContent")
        .and_then(|node| node.text())
        .ok_or(EwsError::Protocol)?;
    let limit = limit.min(32 * 1024 * 1024);
    let mut compact = Vec::new();
    for byte in encoded.bytes().filter(|byte| !byte.is_ascii_whitespace()) {
        if compact.len() > limit.saturating_add(2) / 3 * 4 + 4 {
            return Err(EwsError::TooLarge);
        }
        compact.push(byte);
    }
    let mime = STANDARD.decode(&compact).map_err(|_| EwsError::Protocol)?;
    if mime.len() > limit {
        return Err(EwsError::TooLarge);
    }
    if mime.is_empty() {
        return Err(EwsError::Protocol);
    }
    Ok(mime)
}

/// One result per requested item, in request order. A whole-response error
/// fails the call; an item the server refused (gone, busy) fails only itself.
pub(super) fn parse_previews(
    bytes: &[u8],
    expected: &[&str],
    limit: usize,
) -> Result<Vec<Result<Option<String>, EwsError>>, EwsError> {
    let doc = document(bytes)?;
    let messages = response_messages(&doc, "GetItemResponseMessage")?;
    let responses: Vec<_> = messages.children().filter(Node::is_element).collect();
    if responses.len() != expected.len() {
        return Err(EwsError::Protocol);
    }
    let mut result = Vec::with_capacity(expected.len());
    for (response, expected_id) in responses.into_iter().zip(expected) {
        if let Err(error) = check_response(response, "GetItemResponseMessage") {
            // A mislabelled element is a broken reply, not one refused item.
            if error == EwsError::Protocol {
                return Err(error);
            }
            result.push(Err(error));
            continue;
        }
        let items = child(response, MESSAGES, "Items").ok_or(EwsError::Protocol)?;
        let mut items = items.children().filter(Node::is_element);
        let item = items.next().ok_or(EwsError::Protocol)?;
        if items.next().is_some() {
            return Err(EwsError::Protocol);
        }
        let (id, _) = identifier(item, "ItemId")?;
        if id != *expected_id {
            return Err(EwsError::Protocol);
        }
        result.push(preview_body(item, limit));
    }
    Ok(result)
}

fn preview_body(item: Node<'_, '_>, limit: usize) -> Result<Option<String>, EwsError> {
    let Some(body) = child(item, TYPES, "Body") else {
        return Ok(None);
    };
    if body.attribute("BodyType") != Some("Text") {
        return Err(EwsError::Protocol);
    }
    let preview: String = body
        .text()
        .unwrap_or_default()
        .chars()
        .take(limit.min(4096))
        .collect();
    Ok((!preview.is_empty()).then_some(preview))
}
