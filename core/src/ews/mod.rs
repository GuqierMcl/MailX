//! Exchange adapter. Account selection and durable queue policy belong to the
//! application service; this module owns only HTTPS, NTLMv2 and EWS messages.
pub(crate) mod discovery;
mod http;
mod soap;

use chrono::{DateTime, Utc};
use url::Url;

use crate::mail::MailAddress;
use crate::net::{NetError, NetRoute};
use crate::secrets::SecretValue;
use crate::smtp::{DataStarted, message::SmtpEnvelope};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Endpoint {
    pub url: Url,
}

impl Endpoint {
    pub(crate) fn parse(value: &str) -> Result<Self, EwsError> {
        if value.len() > 2048 || value.chars().any(char::is_control) {
            return Err(EwsError::Protocol);
        }
        let url = Url::parse(value.trim()).map_err(|_| EwsError::Protocol)?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.port() == Some(0)
        {
            return Err(EwsError::Protocol);
        }
        Ok(Self { url })
    }

    pub(crate) fn as_str(&self) -> &str {
        self.url.as_str()
    }

    pub(crate) fn host(&self) -> &str {
        self.url.host_str().expect("validated endpoint")
    }

    pub(crate) fn port(&self) -> u16 {
        self.url.port_or_known_default().expect("HTTPS endpoint")
    }
}

/// Fixed categories and a small whitelist of EWS response codes, never raw
/// peer text, URLs, challenges or passwords.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EwsError {
    Net(NetError),
    Timeout,
    Authentication,
    /// The peer cannot negotiate NTLMv2, directly or inside SPNEGO. A bare
    /// Negotiate header alone never establishes which mechanisms it accepts.
    AuthUnsupported,
    Protocol,
    TooLarge,
    NotFound,
    Server {
        code: String,
    },
    DataMarkFailed,
    /// A delivery request may have reached Exchange. Never replay automatically.
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EwsFolderRole {
    Inbox,
    Sent,
    Drafts,
    Deleted,
    Junk,
}

#[derive(Clone, Debug)]
pub(crate) struct EwsFolder {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub role: Option<EwsFolderRole>,
    pub total: u32,
    pub unread: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct EwsItem {
    pub id: String,
    pub change_key: Option<String>,
    pub internet_message_id: Option<String>,
    pub subject: String,
    pub from: Option<MailAddress>,
    pub to: Vec<MailAddress>,
    pub cc: Vec<MailAddress>,
    pub reply_to: Vec<MailAddress>,
    pub received_at: Option<DateTime<Utc>>,
    pub sent_at: Option<DateTime<Utc>>,
    pub is_read: bool,
    pub is_flagged: bool,
    pub size: u64,
    pub has_attachments: bool,
    pub preview: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Option<String>,
}

pub(crate) struct EwsPage {
    pub items: Vec<EwsItem>,
    /// None only for a server-confirmed last page. Invalid/stuck cursors fail.
    pub next_offset: Option<usize>,
}

pub(crate) struct EwsClient {
    http: http::Connection,
}

impl EwsClient {
    #[cfg(test)]
    pub(crate) async fn test_stream(stream: crate::net::BoxedStream) -> Self {
        let endpoint = Endpoint::parse("https://controlled.example/EWS/Exchange.asmx").unwrap();
        Self {
            http: http::Connection::with_stream(stream, &endpoint)
                .await
                .unwrap(),
        }
    }

    pub(crate) async fn connect(
        route: &NetRoute,
        endpoint: &Endpoint,
        username: &str,
        password: &SecretValue,
    ) -> Result<Self, EwsError> {
        let http = http::Connection::connect(route, endpoint, username, password).await?;
        Ok(Self { http })
    }

    pub(crate) async fn verify(&mut self) -> Result<(), EwsError> {
        let response = self.http.post("GetFolder", soap::probe()).await?;
        soap::check(&response, "GetFolderResponseMessage")
    }

    pub(crate) async fn folders(&mut self) -> Result<Vec<EwsFolder>, EwsError> {
        let mut folders = Vec::new();
        let mut ids = std::collections::HashSet::new();
        let mut offset = 0;
        let mut pages = 0;
        loop {
            if folders.len() >= 10_000 || pages >= 40 {
                return Err(EwsError::TooLarge);
            }
            pages += 1;
            let response = self.http.post("FindFolder", soap::folders(offset)).await?;
            let (page, next) = soap::parse_folders(&response, offset)?;
            for folder in page {
                if !ids.insert(folder.id.clone()) {
                    return Err(EwsError::Protocol);
                }
                folders.push(folder);
            }
            match next {
                Some(next) => offset = next,
                None => break,
            }
        }
        // Roles are server identities, never names (which are localized).
        for (name, role) in [
            ("inbox", EwsFolderRole::Inbox),
            ("sentitems", EwsFolderRole::Sent),
            ("drafts", EwsFolderRole::Drafts),
            ("deleteditems", EwsFolderRole::Deleted),
            ("junkemail", EwsFolderRole::Junk),
        ] {
            let response = self
                .http
                .post("GetFolder", soap::distinguished(name))
                .await?;
            match soap::folder_id(&response) {
                Ok(id) => {
                    if let Some(folder) = folders.iter_mut().find(|folder| folder.id == id) {
                        folder.role = Some(role);
                    }
                }
                // Optional folders may not exist. Transport and malformed XML
                // must still fail the round, rather than publish partial roles.
                Err(EwsError::NotFound) | Err(EwsError::Server { .. }) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(folders)
    }

    /// Metadata only, newest received first, within the caller's sync window.
    pub(crate) async fn items(
        &mut self,
        folder_id: &str,
        since: DateTime<Utc>,
        offset: usize,
        limit: usize,
    ) -> Result<EwsPage, EwsError> {
        if !(1..=500).contains(&limit) || offset > 1_000_000 {
            return Err(EwsError::Protocol);
        }
        let response = self
            .http
            .post("FindItem", soap::items(folder_id, since, offset, limit))
            .await?;
        let mut page = soap::parse_items(&response, offset)?;
        // EWS does not expose recipient collections in FindItem's summary
        // shape. Load them in bounded metadata batches, without body or MIME.
        for items in page.items.chunks_mut(50) {
            let ids: Vec<_> = items.iter().map(|item| item.id.as_str()).collect();
            let response = self
                .http
                .post("GetItem", soap::get_recipients(&ids))
                .await?;
            let recipients = soap::parse_recipients(&response, &ids)?;
            for recipients in recipients {
                if let Some(item) = items.iter_mut().find(|item| item.id == recipients.id) {
                    item.to = recipients.to;
                    item.cc = recipients.cc;
                    item.reply_to = recipients.reply_to;
                }
            }
        }
        Ok(page)
    }

    pub(crate) async fn mime(
        &mut self,
        id: &str,
        change_key: Option<&str>,
        limit: usize,
    ) -> Result<Vec<u8>, EwsError> {
        let response = self
            .http
            .post("GetItem", soap::get_mime(id, change_key))
            .await?;
        soap::parse_mime(&response, limit)
    }

    pub(crate) async fn preview(
        &mut self,
        id: &str,
        change_key: Option<&str>,
        limit: usize,
    ) -> Result<Option<String>, EwsError> {
        let response = self
            .http
            .post("GetItem", soap::get_preview(id, change_key))
            .await?;
        soap::parse_preview(&response, limit)
    }

    pub(crate) async fn set_flags(
        &mut self,
        id: &str,
        _change_key: Option<&str>,
        read: Option<bool>,
        flagged: Option<bool>,
    ) -> Result<(), EwsError> {
        if read.is_none() && flagged.is_none() {
            return Ok(());
        }
        // Only the requested scalar fields are overwritten. AutoResolve would
        // require a fresh ChangeKey; using a cached key can starve later writes
        // after an earlier flag update changes that key.
        let response = self
            .http
            .post("UpdateItem", soap::flags(id, read, flagged))
            .await?;
        soap::check(&response, "UpdateItemResponseMessage")
    }

    pub(crate) async fn delete(
        &mut self,
        id: &str,
        _change_key: Option<&str>,
        permanent: bool,
    ) -> Result<(), EwsError> {
        let response = self
            .http
            .post("DeleteItem", soap::delete(id, permanent))
            .await?;
        soap::check(&response, "DeleteItemResponseMessage")
    }

    pub(crate) async fn send(
        &mut self,
        mime: &[u8],
        envelope: &SmtpEnvelope,
        before_delivery: DataStarted,
    ) -> Result<(), EwsError> {
        let body = soap::send(mime, envelope)?;
        // The callback runs inside post_delivery, after ready() and before the
        // request can reach the socket. Its error aborts without sending.
        let response = self.http.post_delivery(body, before_delivery).await?;
        soap::check(&response, "CreateItemResponseMessage").map_err(|error| match error {
            EwsError::Server { .. } | EwsError::Authentication | EwsError::NotFound => error,
            _ => EwsError::Unknown,
        })
    }
}

#[cfg(test)]
mod tests;
