//! Exchange Web Services: the only channel a local Exchange deployment is
//! guaranteed to expose.
//!
//! Scope and staging: `docs/architecture/ews-backend.md`. This module is being
//! built bottom-up — authentication and transport first, then the operations
//! that map onto the protocol-neutral mail backend. Nothing outside the module
//! calls it yet, so unused pieces are expected until the account model lands.
#![allow(dead_code, reason = "built before the EWS account kind is wired in")]

pub(crate) mod crypto;
pub(crate) mod http;
pub(crate) mod ntlm;
pub(crate) mod soap;

use crate::net::NetRoute;
use crate::secrets::SecretValue;

/// Where one account's EWS endpoint lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Endpoint {
    pub host: String,
    pub port: u16,
    pub path: String,
}

impl Endpoint {
    /// The default endpoint on `host`. A load balancer may publish another path,
    /// which is why it is a field rather than a constant.
    pub(crate) fn new(host: &str) -> Self {
        Self {
            host: host.to_owned(),
            port: 443,
            path: "/EWS/Exchange.asmx".to_owned(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EwsError {
    Http(http::HttpError),
    Soap(soap::SoapError),
}

/// A deep traversal is paged. Cap it so a broken server cursor cannot loop
/// forever (40 pages of 256 folders is far beyond any real mailbox).
const MAX_FOLDER_PAGES: usize = 40;
/// The hierarchy repair asks one shallow `FindFolder` per folder that reports
/// children. Bound it so a pathological mailbox cannot stall a sync round.
const MAX_SHALLOW_QUERIES: usize = 64;

/// Which `FindFolder` response shape this server accepts. `BaseShape=Default`
/// omits `ParentFolderId`, so the first choice asks for it explicitly; a server
/// that rejects the extra `FieldURI` paths still returns parents under
/// `AllProperties`; the bare `Default` shape is the last resort and is paired
/// with a shallow hierarchy repair.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FolderShape {
    Rich,
    AllProperties,
    Plain,
}

impl FolderShape {
    fn request(self, offset: usize) -> String {
        match self {
            Self::Rich => soap::find_folders_page(offset),
            Self::AllProperties => soap::find_folders_all(offset),
            Self::Plain => soap::find_folders_plain(offset),
        }
    }

    fn downgrade(self) -> Option<Self> {
        match self {
            Self::Rich => Some(Self::AllProperties),
            Self::AllProperties => Some(Self::Plain),
            Self::Plain => None,
        }
    }
}

/// True when the server clearly has a tree but the listing carries no usable
/// parent links: e.g. a deployment that ignores `folder:ParentFolderId` or
/// rejects every richer shape. A genuinely flat mailbox is left alone.
fn needs_parent_repair(folders: &[soap::Folder]) -> bool {
    let ids: std::collections::HashSet<&str> =
        folders.iter().map(|folder| folder.id.as_str()).collect();
    let linked = folders.iter().any(|folder| {
        folder
            .parent_id
            .as_deref()
            .is_some_and(|parent| ids.contains(parent))
    });
    let has_children = folders
        .iter()
        .any(|folder| folder.child_folder_count.unwrap_or(0) > 0);
    !linked && has_children
}

/// Rebuilds the tree without trusting `ParentFolderId`: each folder is asked
/// for its direct children, so the parent is known from the request itself.
/// Best effort — a refusal leaves the listing flat but keeps the account
/// syncing instead of failing the whole round.
async fn repair_parents(http: &mut http::EwsHttp, folders: &mut [soap::Folder]) {
    let indices: std::collections::HashMap<String, usize> = folders
        .iter()
        .enumerate()
        .map(|(index, folder)| (folder.id.clone(), index))
        .collect();
    for folder in folders.iter_mut() {
        folder.parent_id = None;
    }
    let candidates: Vec<String> = folders
        .iter()
        .filter(|folder| folder.mail_folder && folder.child_folder_count.unwrap_or(0) > 0)
        .take(MAX_SHALLOW_QUERIES)
        .map(|folder| folder.id.clone())
        .collect();
    for parent in candidates {
        let mut offset = 0usize;
        for _ in 0..MAX_FOLDER_PAGES {
            let request = soap::find_folder_children(&parent, offset);
            let response = match http.post("FindFolder", &request).await {
                Ok(response) => response,
                Err(_) => break,
            };
            let page = match soap::parse_folders_page(&response, offset) {
                Ok(page) => page,
                Err(_) => break,
            };
            let count = page.folders.len();
            for child in page.folders {
                if let Some(&index) = indices.get(&child.id)
                    && folders[index].parent_id.is_none()
                {
                    folders[index].parent_id = Some(parent.clone());
                }
            }
            match page.next_offset {
                Some(next) if next > offset && count > 0 => offset = next,
                _ => break,
            }
        }
    }
}

/// An authenticated EWS session. It owns one connection: dropping it closes the
/// NTLM context, and a new one has to authenticate from the start.
pub(crate) struct EwsClient {
    http: http::EwsHttp,
    /// Once a server has rejected the full `FindItem` property set, the rest of
    /// this session goes straight to the conservative set.
    basic_item_fields: bool,
    /// The richest `FindFolder` shape this server has accepted so far.
    folder_shape: FolderShape,
}

impl EwsClient {
    pub(crate) async fn connect(
        route: &NetRoute,
        endpoint: &Endpoint,
        user: &str,
        password: &SecretValue,
    ) -> Result<Self, EwsError> {
        let credentials = ntlm::Credentials::parse(user, password);
        let http = http::EwsHttp::open(
            route,
            &endpoint.host,
            endpoint.port,
            &endpoint.path,
            &credentials,
        )
        .await
        .map_err(EwsError::Http)?;
        Ok(Self {
            http,
            basic_item_fields: false,
            folder_shape: FolderShape::Rich,
        })
    }

    /// Every folder visible to this mailbox, deep from the message-folder root.
    /// `FindFolder` returns a flat paged list, so this walks the pages, drops
    /// duplicate ids and stops when the server says the last item is included.
    /// When the response carries no usable parent links, a bounded shallow
    /// pass rebuilds the hierarchy from the requests themselves.
    pub(crate) async fn folders(&mut self) -> Result<Vec<soap::Folder>, EwsError> {
        let mut all = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut offset = 0usize;
        for _ in 0..MAX_FOLDER_PAGES {
            let mut shape = self.folder_shape;
            let response = loop {
                match self.http.post("FindFolder", &shape.request(offset)).await {
                    Ok(response) => break response,
                    // A strictly validating deployment can reject the extra
                    // FieldURI paths while deserializing the shape. Step down
                    // one shape at a time and remember it for later pages.
                    Err(error) if rejected_property_set(&error) => match shape.downgrade() {
                        Some(next) => {
                            self.folder_shape = next;
                            shape = next;
                        }
                        None => return Err(EwsError::Http(error)),
                    },
                    Err(error) => return Err(EwsError::Http(error)),
                }
            };
            let page = soap::parse_folders_page(&response, offset).map_err(EwsError::Soap)?;
            let count = page.folders.len();
            for folder in page.folders {
                if seen.insert(folder.id.clone()) {
                    all.push(folder);
                }
            }
            match page.next_offset {
                Some(next) if next > offset && count > 0 => offset = next,
                _ => break,
            }
        }
        if needs_parent_repair(&all) {
            repair_parents(&mut self.http, &mut all).await;
        }
        Ok(all)
    }

    /// Best-effort lookup of the real `FolderId` behind each distinguished
    /// folder (`inbox`, `sentitems`, ...). Exchange mailboxes can contain empty
    /// look-alike folders left by a locale switch or an old client; only this
    /// operation says which folder actually holds sent mail and drafts.
    ///
    /// A deployment that refuses `GetFolder` or answers with a shape we cannot
    /// read simply gets an empty vector: role guessing by name still works, and
    /// an optional refinement must never break the folder listing.
    pub(crate) async fn distinguished_folder_ids(&mut self, names: &[&str]) -> Vec<Option<String>> {
        let request = soap::get_distinguished_folders(names);
        let response = match self.http.post("GetFolder", &request).await {
            Ok(response) => response,
            Err(_) => return Vec::new(),
        };
        let ids = soap::parse_distinguished_folders(&response);
        // A partial answer would shift every later id onto the wrong role.
        (ids.len() == names.len())
            .then_some(ids)
            .unwrap_or_default()
    }

    /// One page of messages in `folder_id`, newest first as EWS returns them.
    ///
    /// Some on-premises deployments reject the whole request — with a top-level
    /// SOAP fault over HTTP 500 rather than a per-item error — when one of the
    /// newer `AdditionalProperties` paths is unknown to their schema. Retrying
    /// once with the original property set keeps those accounts syncing; the
    /// reader treats every field it does not receive as absent.
    pub(crate) async fn items(
        &mut self,
        folder_id: &str,
        max_entries: usize,
    ) -> Result<Vec<soap::Item>, EwsError> {
        let request = if self.basic_item_fields {
            soap::find_items_basic(folder_id, max_entries)
        } else {
            soap::find_items(folder_id, max_entries)
        };
        let response = match self.http.post("FindItem", &request).await {
            Ok(response) => response,
            Err(error) if !self.basic_item_fields && rejected_property_set(&error) => {
                self.basic_item_fields = true;
                let fallback = soap::find_items_basic(folder_id, max_entries);
                self.http
                    .post("FindItem", &fallback)
                    .await
                    .map_err(EwsError::Http)?
            }
            Err(error) => return Err(EwsError::Http(error)),
        };
        soap::parse_items(&response).map_err(EwsError::Soap)
    }

    /// The raw MIME source of one message.
    pub(crate) async fn mime(
        &mut self,
        item_id: &str,
        change_key: Option<&str>,
    ) -> Result<Vec<u8>, EwsError> {
        let request = soap::get_item_mime(item_id, change_key);
        let response = self
            .http
            .post("GetItem", &request)
            .await
            .map_err(EwsError::Http)?;
        soap::parse_mime(&response).map_err(EwsError::Soap)
    }

    /// Writes back the read / flag state of one message.
    pub(crate) async fn set_flags(
        &mut self,
        item_id: &str,
        change_key: Option<&str>,
        read: Option<bool>,
        flagged: Option<bool>,
    ) -> Result<(), EwsError> {
        let request = soap::update_flags(item_id, change_key, read, flagged);
        let response = self
            .http
            .post("UpdateItem", &request)
            .await
            .map_err(EwsError::Http)?;
        soap::response_ok(&response, "UpdateItemResponseMessage").map_err(EwsError::Soap)
    }

    /// Moves one message to Deleted Items.
    pub(crate) async fn delete(
        &mut self,
        item_id: &str,
        change_key: Option<&str>,
    ) -> Result<(), EwsError> {
        let request = soap::delete_item(item_id, change_key);
        let response = self
            .http
            .post("DeleteItem", &request)
            .await
            .map_err(EwsError::Http)?;
        soap::response_ok(&response, "DeleteItemResponseMessage").map_err(EwsError::Soap)
    }

    /// Sends a raw MIME message and keeps a copy in Sent Items.
    pub(crate) async fn send(&mut self, mime: &[u8]) -> Result<(), EwsError> {
        let request = soap::create_and_send(mime);
        let response = self
            .http
            .post("CreateItem", &request)
            .await
            .map_err(EwsError::Http)?;
        soap::response_ok(&response, "CreateItemResponseMessage").map_err(EwsError::Soap)
    }
}

/// Whether a failed `FindItem` is the kind a leaner property set can repair.
/// Exchange answers an unrecognised `FieldURI` with a top-level SOAP fault over
/// HTTP 500; a 4xx, a transport failure or a different 5xx has another cause
/// and must surface unchanged instead of being retried away.
fn rejected_property_set(error: &http::HttpError) -> bool {
    matches!(error, http::HttpError::UnexpectedStatus(500, _))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_endpoint_is_the_standard_path_on_443() {
        let endpoint = Endpoint::new("mail.example.com");
        assert_eq!(endpoint.port, 443);
        assert_eq!(endpoint.path, "/EWS/Exchange.asmx");
    }

    #[test]
    fn folder_shape_stepdown_never_skips_a_fallback() {
        assert_eq!(
            FolderShape::Rich.downgrade(),
            Some(FolderShape::AllProperties)
        );
        assert_eq!(
            FolderShape::AllProperties.downgrade(),
            Some(FolderShape::Plain)
        );
        assert_eq!(FolderShape::Plain.downgrade(), None);
        assert!(!FolderShape::AllProperties.request(0).contains("FieldURI"));
        assert!(!FolderShape::Plain.request(0).contains("FieldURI"));
    }

    fn fake_folder(id: &str, parent: Option<&str>, children: Option<u32>) -> soap::Folder {
        soap::Folder {
            id: id.into(),
            parent_id: parent.map(Into::into),
            display_name: id.into(),
            total_count: 0,
            unread_count: 0,
            child_folder_count: children,
            folder_class: None,
            mail_folder: true,
        }
    }

    #[test]
    fn parent_repair_only_runs_when_the_tree_is_lost() {
        // The normal case: the server answered our explicit ParentFolderId.
        assert!(!needs_parent_repair(&[
            fake_folder("in", None, Some(2)),
            fake_folder("kid", Some("in"), None),
        ]));
        // A genuinely flat mailbox has no children to query.
        assert!(!needs_parent_repair(&[
            fake_folder("in", None, Some(0)),
            fake_folder("archive", None, Some(0)),
        ]));
        // Children exist but no parent link resolves: the flattened sidebar.
        assert!(needs_parent_repair(&[
            fake_folder("in", None, Some(2)),
            fake_folder("kid", None, None),
        ]));
        // An id pointing outside the listing is not a usable link either.
        assert!(needs_parent_repair(&[
            fake_folder("in", Some("msgroot"), Some(1)),
            fake_folder("kid", Some("msgroot"), None),
        ]));
    }

    #[test]
    fn only_a_server_side_property_rejection_is_retried() {
        assert!(rejected_property_set(&http::HttpError::UnexpectedStatus(
            500,
            "HTTP 500 · text/xml · [FindItem]：ErrorInvalidRequest".into(),
        )));
        assert!(!rejected_property_set(&http::HttpError::UnexpectedStatus(
            502,
            "bad gateway".into(),
        )));
        assert!(!rejected_property_set(&http::HttpError::Rejected));
        assert!(!rejected_property_set(&http::HttpError::Transport));
        assert!(!rejected_property_set(&http::HttpError::NoNtlm));
    }

    #[test]
    fn ews_errors_stay_split_between_http_and_soap() {
        assert_eq!(
            EwsError::Soap(soap::SoapError::Malformed),
            EwsError::Soap(soap::SoapError::Malformed)
        );
        assert_eq!(
            EwsError::Soap(soap::SoapError::Server),
            EwsError::Soap(soap::SoapError::Server)
        );
        assert_eq!(
            EwsError::Http(http::HttpError::Rejected),
            EwsError::Http(http::HttpError::Rejected)
        );
    }

    #[test]
    fn every_soap_operation_builds_an_envelope() {
        for request in [
            soap::find_items("inbox", 10),
            soap::get_item_mime("id-1", None),
            soap::update_flags("id-1", None, Some(true), None),
            soap::delete_item("id-1", None),
            soap::create_and_send(b"hello"),
        ] {
            assert!(request.contains("<soap:Envelope"), "not an envelope");
            assert!(request.ends_with("</soap:Envelope>"));
        }
    }

    /// Explicit opt-in against a real mailbox. Set `MAILX_EWS_HOST`,
    /// `MAILX_EWS_USER` (`DOMAIN\user`) and `MAILX_EWS_PASSWORD`, then:
    ///
    /// ```text
    /// cargo test -p mailx-core --lib a_real_exchange -- --ignored --nocapture
    /// ```
    ///
    /// It prints stage results only, never ids, folder names, hosts or
    /// credentials. It also runs a `FindItem` in the busiest folder: that is
    /// the operation an unknown `FieldURI` breaks with a 500 /
    /// `ErrorInvalidRequest`. A server that does not accept the full property
    /// set is retried with the conservative set.
    #[tokio::test]
    #[ignore = "needs a real Exchange account in MAILX_EWS_HOST/USER/PASSWORD"]
    async fn a_real_exchange_lists_its_folders() {
        let host = std::env::var("MAILX_EWS_HOST").expect("MAILX_EWS_HOST");
        let user = std::env::var("MAILX_EWS_USER").expect("MAILX_EWS_USER");
        let password =
            SecretValue::new(std::env::var("MAILX_EWS_PASSWORD").expect("MAILX_EWS_PASSWORD"));
        let route = NetRoute::default();
        let endpoint = Endpoint::new(&host);
        let mut client = EwsClient::connect(&route, &endpoint, &user, &password)
            .await
            .unwrap_or_else(|error| panic!("[live] sign-in: FAILED ({})", live_error(&error)));
        println!("[live] sign-in: OK");
        let folders = client
            .folders()
            .await
            .unwrap_or_else(|error| panic!("[live] FindFolder: FAILED ({})", live_error(&error)));
        assert!(!folders.is_empty(), "a mailbox always has folders");
        println!("[live] FindFolder: OK ({} folders)", folders.len());
        let busiest = folders
            .iter()
            .max_by_key(|folder| folder.total_count)
            .expect("at least one folder");
        let items = client
            .items(&busiest.id, 5)
            .await
            .unwrap_or_else(|error| panic!("[live] FindItem: FAILED ({})", live_error(&error)));
        println!("[live] FindItem: OK ({} items)", items.len());
    }

    /// A short, redacted label for the live probe. It never carries the
    /// response body, the host, or any credential.
    fn live_error(error: &EwsError) -> String {
        match error {
            EwsError::Http(http::HttpError::UnexpectedStatus(status, _)) => {
                format!("HTTP {status}")
            }
            EwsError::Http(http::HttpError::Rejected) => "credentials rejected".into(),
            EwsError::Http(http::HttpError::NoNtlm) => "NTLM not offered".into(),
            EwsError::Http(http::HttpError::Timeout) => "timeout".into(),
            EwsError::Http(http::HttpError::Transport) => "transport".into(),
            EwsError::Http(http::HttpError::Malformed) => "malformed response".into(),
            EwsError::Http(http::HttpError::TooLarge) => "response too large".into(),
            EwsError::Http(http::HttpError::Net(net_error)) => match net_error {
                crate::net::NetError::Connect(_) => "network/connect",
                crate::net::NetError::Proxy(_) => "network/proxy",
                crate::net::NetError::Tls(_) => "network/tls",
                crate::net::NetError::Timeout => "network/timeout",
            }
            .into(),
            EwsError::Soap(soap::SoapError::Malformed) => "malformed SOAP".into(),
            EwsError::Soap(soap::SoapError::Server) => "SOAP server error".into(),
        }
    }

    /// One HTTP request off the wire: headers, then the body Content-Length
    /// announces. Kept local so the folder-paging test is self-contained.
    async fn read_request(stream: &mut tokio::net::TcpStream) -> (String, Vec<u8>) {
        use tokio::io::AsyncReadExt;
        let mut buffer = Vec::new();
        let mut chunk = [0u8; 2048];
        loop {
            let read = stream.read(&mut chunk).await.expect("read");
            assert!(read > 0, "client closed before sending a request");
            buffer.extend_from_slice(&chunk[..read]);
            let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n") else {
                continue;
            };
            let head = String::from_utf8_lossy(&buffer[..end]).to_string();
            let length = head
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())?
                })
                .unwrap_or(0);
            if buffer.len() >= end + 4 + length {
                return (head, buffer[end + 4..end + 4 + length].to_vec());
            }
        }
    }

    async fn reply(stream: &mut tokio::net::TcpStream, status: &str, extra: &str, body: &str) {
        use tokio::io::AsyncWriteExt;
        let response = format!(
            "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).await.expect("write");
    }

    fn type2_challenge() -> String {
        use base64::Engine;
        let target_info = [0x02u8, 0, 0, 0, 0, 0, 0, 0];
        let mut message = Vec::new();
        message.extend_from_slice(b"NTLMSSP\0");
        message.extend_from_slice(&2u32.to_le_bytes());
        message.extend_from_slice(&[0u8; 8]);
        message.extend_from_slice(&crate::ews::ntlm::NEGOTIATE_FLAGS.to_le_bytes());
        message.extend_from_slice(&[0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef]);
        message.extend_from_slice(&[0u8; 8]);
        message.extend_from_slice(&(target_info.len() as u16).to_le_bytes());
        message.extend_from_slice(&(target_info.len() as u16).to_le_bytes());
        message.extend_from_slice(&48u32.to_le_bytes());
        message.extend_from_slice(&target_info);
        base64::engine::general_purpose::STANDARD.encode(&message)
    }

    fn folder_page(id: &str, last: bool, next: usize) -> String {
        format!(
            concat!(
                r#"<?xml version="1.0"?><s:Envelope><s:Body>"#,
                r#"<m:FindFolderResponse><m:ResponseMessages>"#,
                r#"<m:FindFolderResponseMessage ResponseClass="Success">"#,
                r#"<m:ResponseCode>NoError</m:ResponseCode>"#,
                r#"<m:RootFolder IndexedPagingOffset="{next}" IncludesLastItemInRange="{last}">"#,
                r#"<t:Folders><t:Folder><t:FolderId Id="{id}"/>"#,
                r#"<t:ParentFolderId Id="msgroot"/><t:FolderClass>IPF.Note</t:FolderClass>"#,
                r#"<t:DisplayName>Folder {id}</t:DisplayName><t:TotalCount>2</t:TotalCount>"#,
                r#"<t:UnreadCount>1</t:UnreadCount></t:Folder></t:Folders>"#,
                r#"</m:RootFolder></m:FindFolderResponseMessage></m:ResponseMessages>"#,
                r#"</m:FindFolderResponse></s:Body></s:Envelope>"#,
            ),
            next = next,
            last = last,
            id = id,
        )
    }

    #[tokio::test]
    async fn folder_listing_walks_every_page_on_one_connection() {
        use tokio::net::{TcpListener, TcpStream};

        use crate::ews::http::EwsHttp;
        use crate::ews::ntlm::Credentials;
        use crate::net::BoxedStream;

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let challenge = type2_challenge();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_request(&mut stream).await;
            reply(
                &mut stream,
                "401 Unauthorized",
                "WWW-Authenticate: Negotiate\r\nWWW-Authenticate: NTLM\r\n",
                "",
            )
            .await;
            read_request(&mut stream).await;
            reply(
                &mut stream,
                "401 Unauthorized",
                &format!("WWW-Authenticate: NTLM {challenge}\r\n"),
                "",
            )
            .await;
            // Type 3 carries a real FindFolder request; its body must be drained.
            read_request(&mut stream).await;
            reply(
                &mut stream,
                "200 OK",
                "Content-Type: text/xml\r\n",
                "<soap:Envelope><soap:Body/></soap:Envelope>",
            )
            .await;
            let (_, first) = read_request(&mut stream).await;
            let first = String::from_utf8_lossy(&first).to_string();
            assert!(first.contains(r#"Offset="0""#), "first page: {first}");
            // The whole point of the fix: Default never returns ParentFolderId.
            assert!(
                first.contains(r#"<t:FieldURI FieldURI="folder:ParentFolderId"/>"#),
                "first page must ask for the parent: {first}"
            );
            reply(
                &mut stream,
                "200 OK",
                "Content-Type: text/xml\r\n",
                &folder_page("a", false, 256),
            )
            .await;
            let (_, second) = read_request(&mut stream).await;
            let second = String::from_utf8_lossy(&second).to_string();
            assert!(second.contains(r#"Offset="256""#), "second page: {second}");
            reply(
                &mut stream,
                "200 OK",
                "Content-Type: text/xml\r\n",
                &folder_page("b", true, 257),
            )
            .await;
        });

        let stream = Box::new(TcpStream::connect(address).await.unwrap()) as BoxedStream;
        let password = SecretValue::new("synthetic".to_owned());
        let credentials = Credentials::parse("Domain\\User", &password);
        let http = EwsHttp::with_stream(
            stream,
            "mail.example.com",
            "/EWS/Exchange.asmx",
            &credentials,
        )
        .await
        .expect("authentication succeeds");
        let mut client = EwsClient {
            http,
            basic_item_fields: false,
            folder_shape: FolderShape::Rich,
        };
        let folders = client.folders().await.expect("both pages are read");
        assert_eq!(folders.len(), 2);
        assert_eq!(folders[0].id, "a");
        assert_eq!(folders[1].id, "b");
        assert_eq!(folders[0].unread_count, 1);
        server.await.unwrap();
    }

    /// A deep response with no `ParentFolderId` (bare `Default` shape) plus a
    /// `ChildFolderCount` is exactly the flattened sidebar the user reported.
    fn flat_listing_with_children() -> String {
        concat!(
            r#"<?xml version="1.0"?><s:Envelope><s:Body>"#,
            r#"<m:FindFolderResponse><m:ResponseMessages>"#,
            r#"<m:FindFolderResponseMessage ResponseClass="Success">"#,
            r#"<m:RootFolder IncludesLastItemInRange="true"><t:Folders>"#,
            r#"<t:Folder><t:FolderId Id="in"/><t:DisplayName>收件箱</t:DisplayName>"#,
            r#"<t:TotalCount>9</t:TotalCount><t:UnreadCount>2</t:UnreadCount>"#,
            r#"<t:ChildFolderCount>1</t:ChildFolderCount></t:Folder>"#,
            r#"<t:Folder><t:FolderId Id="kid"/><t:DisplayName>项目</t:DisplayName>"#,
            r#"<t:TotalCount>3</t:TotalCount><t:UnreadCount>0</t:UnreadCount>"#,
            r#"<t:ChildFolderCount>0</t:ChildFolderCount></t:Folder>"#,
            r#"</t:Folders></m:RootFolder></m:FindFolderResponseMessage>"#,
            r#"</m:ResponseMessages></m:FindFolderResponse></s:Body></s:Envelope>"#,
        )
        .to_owned()
    }

    fn shallow_children_listing() -> String {
        concat!(
            r#"<?xml version="1.0"?><s:Envelope><s:Body>"#,
            r#"<m:FindFolderResponse><m:ResponseMessages>"#,
            r#"<m:FindFolderResponseMessage ResponseClass="Success">"#,
            r#"<m:RootFolder IncludesLastItemInRange="true"><t:Folders>"#,
            r#"<t:Folder><t:FolderId Id="kid"/><t:DisplayName>项目</t:DisplayName></t:Folder>"#,
            r#"</t:Folders></m:RootFolder></m:FindFolderResponseMessage>"#,
            r#"</m:ResponseMessages></m:FindFolderResponse></s:Body></s:Envelope>"#,
        )
        .to_owned()
    }

    /// `GetFolder` is only a refinement. A deployment that refuses it must not
    /// cost the account its folder listing (or poison the shared connection).
    #[tokio::test]
    async fn a_rejected_get_folder_does_not_break_the_listing() {
        use tokio::net::{TcpListener, TcpStream};

        use crate::ews::http::EwsHttp;
        use crate::ews::ntlm::Credentials;
        use crate::net::BoxedStream;

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let challenge = type2_challenge();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_request(&mut stream).await;
            reply(
                &mut stream,
                "401 Unauthorized",
                "WWW-Authenticate: Negotiate\r\nWWW-Authenticate: NTLM\r\n",
                "",
            )
            .await;
            read_request(&mut stream).await;
            reply(
                &mut stream,
                "401 Unauthorized",
                &format!("WWW-Authenticate: NTLM {challenge}\r\n"),
                "",
            )
            .await;
            read_request(&mut stream).await;
            reply(
                &mut stream,
                "200 OK",
                "Content-Type: text/xml\r\n",
                "<soap:Envelope><soap:Body/></soap:Envelope>",
            )
            .await;
            let (_, get_folder) = read_request(&mut stream).await;
            assert!(String::from_utf8_lossy(&get_folder).contains("GetFolder"));
            reply(
                &mut stream,
                "500 Internal Server Error",
                "Content-Type: text/xml\r\n",
                "<fault/>",
            )
            .await;
            let (_, find_folder) = read_request(&mut stream).await;
            assert!(String::from_utf8_lossy(&find_folder).contains("FindFolder"));
            reply(
                &mut stream,
                "200 OK",
                "Content-Type: text/xml\r\n",
                &folder_page("a", true, 1),
            )
            .await;
        });

        let stream = Box::new(TcpStream::connect(address).await.unwrap()) as BoxedStream;
        let password = SecretValue::new("synthetic".to_owned());
        let credentials = Credentials::parse("Domain\\User", &password);
        let http = EwsHttp::with_stream(
            stream,
            "mail.example.com",
            "/EWS/Exchange.asmx",
            &credentials,
        )
        .await
        .expect("authentication succeeds");
        let mut client = EwsClient {
            http,
            basic_item_fields: false,
            folder_shape: FolderShape::Rich,
        };
        assert!(
            client
                .distinguished_folder_ids(&["inbox", "sentitems"])
                .await
                .is_empty()
        );
        let folders = client.folders().await.expect("the listing still works");
        assert_eq!(folders.len(), 1);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_listing_without_parent_ids_is_rebuilt_from_shallow_queries() {
        use tokio::net::{TcpListener, TcpStream};

        use crate::ews::http::EwsHttp;
        use crate::ews::ntlm::Credentials;
        use crate::net::BoxedStream;

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let challenge = type2_challenge();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            read_request(&mut stream).await;
            reply(
                &mut stream,
                "401 Unauthorized",
                "WWW-Authenticate: Negotiate\r\nWWW-Authenticate: NTLM\r\n",
                "",
            )
            .await;
            read_request(&mut stream).await;
            reply(
                &mut stream,
                "401 Unauthorized",
                &format!("WWW-Authenticate: NTLM {challenge}\r\n"),
                "",
            )
            .await;
            read_request(&mut stream).await;
            reply(
                &mut stream,
                "200 OK",
                "Content-Type: text/xml\r\n",
                "<soap:Envelope><soap:Body/></soap:Envelope>",
            )
            .await;
            let (_, deep) = read_request(&mut stream).await;
            assert!(
                String::from_utf8_lossy(&deep).contains("folder:ParentFolderId"),
                "the deep page still asks for the parent"
            );
            reply(
                &mut stream,
                "200 OK",
                "Content-Type: text/xml\r\n",
                &flat_listing_with_children(),
            )
            .await;
            let (_, shallow) = read_request(&mut stream).await;
            let shallow = String::from_utf8_lossy(&shallow).to_string();
            assert!(shallow.contains(r#"Traversal="Shallow""#), "{shallow}");
            assert!(shallow.contains(r#"<t:FolderId Id="in"/>"#), "{shallow}");
            reply(
                &mut stream,
                "200 OK",
                "Content-Type: text/xml\r\n",
                &shallow_children_listing(),
            )
            .await;
        });

        let stream = Box::new(TcpStream::connect(address).await.unwrap()) as BoxedStream;
        let password = SecretValue::new("synthetic".to_owned());
        let credentials = Credentials::parse("Domain\\User", &password);
        let http = EwsHttp::with_stream(
            stream,
            "mail.example.com",
            "/EWS/Exchange.asmx",
            &credentials,
        )
        .await
        .expect("authentication succeeds");
        let mut client = EwsClient {
            http,
            basic_item_fields: false,
            folder_shape: FolderShape::Rich,
        };
        let folders = client.folders().await.expect("folders are read");
        assert_eq!(folders.len(), 2);
        assert_eq!(folders[0].parent_id, None, "the Inbox is a root folder");
        assert_eq!(
            folders[1].parent_id.as_deref(),
            Some("in"),
            "the child is nested again even though the response had no parents"
        );
        server.await.unwrap();
    }
}
