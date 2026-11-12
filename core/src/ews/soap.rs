//! SOAP envelopes and the small, bounded reader that goes with them.
//!
//! Responses come from a server, so nothing here trusts them: a `DOCTYPE` is
//! refused outright (no DTD processing means no entity expansion), every read is
//! an offset-bounded slice, and the only shapes understood are the ones this
//! module asks for. Element prefixes (`t:`, `m:`) are matched by local name
//! because Exchange and its load balancers do not always agree on them.

/// Message namespace, used for the body element of each request.
const MESSAGES: &str = "http://schemas.microsoft.com/exchange/services/2006/messages";
/// Type namespace, used for every value type.
const TYPES: &str = "http://schemas.microsoft.com/exchange/services/2006/types";
/// The schema version MailX speaks. Exchange 2013 answers this and newer servers
/// accept it too.
const SERVER_VERSION: &str = "Exchange2013";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SoapError {
    /// The response carries a DTD or is not something this reader accepts.
    Malformed,
    /// `ResponseClass="Error"`; the message stays on the server side.
    Server,
}

/// One folder as `FindFolder` reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Folder {
    pub id: String,
    pub display_name: String,
    pub total_count: u32,
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
pub(crate) fn find_folders() -> String {
    envelope(concat!(
        r#"<m:FindFolder Traversal="Deep">"#,
        r#"<m:FolderShape><t:BaseShape>Default</t:BaseShape></m:FolderShape>"#,
        r#"<m:IndexedPageFolderView MaxEntriesReturned="256" Offset="0" BasePoint="Beginning"/>"#,
        r#"<m:ParentFolderIds><t:DistinguishedFolderId Id="msgfolderroot"/></m:ParentFolderIds>"#,
        r#"</m:FindFolder>"#,
    ))
}

/// Rejects anything this reader must not walk: a DTD, or a body that is not an
/// envelope at all.
fn checked(xml: &str) -> Result<&str, SoapError> {
    let upper = xml.to_ascii_uppercase();
    if upper.contains("<!DOCTYPE") || upper.contains("<!ENTITY") {
        return Err(SoapError::Malformed);
    }
    if !upper.contains("ENVELOPE") {
        return Err(SoapError::Malformed);
    }
    Ok(xml)
}

/// Reads the folder list out of a `FindFolder` response.
pub(crate) fn parse_folders(xml: &str) -> Result<Vec<Folder>, SoapError> {
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
    let mut folders = Vec::new();
    for name in [
        "Folder",
        "CalendarFolder",
        "ContactsFolder",
        "TasksFolder",
        "SearchFolder",
    ] {
        for block in blocks(root, name) {
            let Some(id) = blocks(block, "FolderId").first().copied() else {
                continue;
            };
            let Some(id) = attribute(id, "Id") else {
                continue;
            };
            folders.push(Folder {
                id,
                display_name: text(block, "DisplayName").unwrap_or_default(),
                total_count: text(block, "TotalCount")
                    .and_then(|count| count.parse().ok())
                    .unwrap_or(0),
            });
        }
    }
    Ok(folders)
}

/// The element text of `<…:name>` in `xml`, first match.
fn text(xml: &str, name: &str) -> Option<String> {
    let block = blocks(xml, name).into_iter().next()?;
    let open_end = block.find('>')? + 1;
    let close = block.rfind("</")?;
    (open_end <= close).then(|| unescape(&block[open_end..close]))
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
    fn markup_in_a_folder_name_cannot_break_the_reader() {
        let hostile = RESPONSE.replace("收件箱", "&lt;script&gt;alert(1)&lt;/script&gt;");
        let folders = parse_folders(&hostile).unwrap();
        assert_eq!(folders[0].display_name, "<script>alert(1)</script>");
    }
}
