//! Read-only IMAP operations for folder/header synchronization and body loading.

mod body;
pub(crate) mod utf7;
pub(crate) use body::open_body_session;
#[cfg(test)]
pub(crate) use body::test_body_session;

use std::fmt;
use std::future::Future;
use std::time::Duration;

use async_imap::Client;
use async_imap::error::Error as ImapProtocolError;
use async_imap::imap_proto::{
    AttributeValue, MailboxDatum, MessageSection, Response, ResponseCode, SectionPath, Status,
    StatusAttribute,
};
use async_imap::types::{Flag, NameAttribute};
use chrono::{DateTime, FixedOffset, NaiveDate};
use tokio::time::timeout;

use crate::accounts::types::TlsMode;
use crate::net::{self, BoxedStream, NetError, NetRoute};
use crate::secrets::SecretValue;

/// Upper bound for one command round trip (docs/architecture/mail-sync.md §2);
/// a server that stalls longer is treated as gone.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// Server and credentials for one IMAP login.
#[derive(Debug)]
pub(crate) struct ImapLogin {
    pub host: String,
    pub port: u16,
    pub tls: TlsMode,
    pub username: String,
    pub password: SecretValue,
}

/// A folder as the server lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemoteFolder {
    /// Name exactly as sent by the server (modified UTF-7); used to address it.
    pub name: String,
    pub delimiter: Option<String>,
    /// LIST attributes such as `\Noselect` or `\Sent`, as the server wrote them.
    pub attributes: Vec<String>,
    /// LIST discovery does not fetch counts; STATUS is queried per folder.
    pub status: Option<FolderStatus>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FolderStatus {
    pub messages: u32,
    pub unseen: u32,
    pub uid_next: Option<u32>,
    pub uid_validity: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ImapError {
    Net(NetError),
    /// The server rejected the username or password.
    Authentication(String),
    /// The server does not support a required step, such as STARTTLS.
    Unsupported(String),
    /// The server answered with something the client could not use.
    Protocol(String),
    /// A completed tagged NO/BAD, with all response data consumed.
    Rejected,
    /// A completed command whose mailbox data is unusable; connection is healthy.
    Mailbox(String),
    /// A declared or received body exceeds the bounded message budget.
    TooLarge,
    Timeout,
}

impl fmt::Display for ImapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Net(error) => error.fmt(formatter),
            Self::Authentication(reason) => write!(formatter, "login rejected: {reason}"),
            Self::Unsupported(reason) => write!(formatter, "unsupported by server: {reason}"),
            Self::Protocol(reason) => write!(formatter, "protocol error: {reason}"),
            Self::Rejected => formatter.write_str("protocol error: IMAP command failed"),
            Self::Mailbox(reason) => write!(formatter, "mailbox response error: {reason}"),
            Self::TooLarge => formatter.write_str("message exceeds size limit"),
            Self::Timeout => formatter.write_str("server stopped responding"),
        }
    }
}

impl ImapError {
    pub(crate) fn session_usable(&self) -> bool {
        matches!(self, Self::Rejected | Self::Mailbox(_))
    }
}

impl From<NetError> for ImapError {
    fn from(error: NetError) -> Self {
        Self::Net(error)
    }
}

impl From<ImapProtocolError> for ImapError {
    fn from(_error: ImapProtocolError) -> Self {
        // NO/BAD, parser errors and I/O wrappers may contain server-controlled
        // header or credential data. Never forward their Display/Debug output.
        Self::Protocol("IMAP command failed".into())
    }
}

/// Names the failing command and a coarse cause without forwarding any
/// server-controlled text: the library's parse errors embed the raw response.
fn read_failure(command: &str, error: &std::io::Error) -> ImapError {
    use std::io::ErrorKind;
    let verb = command
        .split_whitespace()
        .take(2)
        .filter(|word| word.chars().all(|c| c.is_ascii_uppercase()))
        .collect::<Vec<_>>()
        .join(" ");
    let cause = match error.kind() {
        // The library reports undecodable responses as `io::ErrorKind::Other`.
        ErrorKind::Other => "server response could not be parsed",
        ErrorKind::InvalidData => "invalid data in the connection stream",
        ErrorKind::UnexpectedEof => "connection ended in the middle of a response",
        ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted | ErrorKind::BrokenPipe => {
            "connection was reset by the server or network"
        }
        ErrorKind::OutOfMemory => "response exceeded the buffer limit",
        _ => "network error while reading response",
    };
    ImapError::Protocol(if verb.is_empty() {
        cause.into()
    } else {
        format!("{verb}: {cause}")
    })
}

async fn step<T, E: Into<ImapError>>(
    future: impl Future<Output = Result<T, E>>,
) -> Result<T, ImapError> {
    timeout(COMMAND_TIMEOUT, future)
        .await
        .map_err(|_| ImapError::Timeout)?
        .map_err(Into::into)
}

/// An authenticated connection; every network operation has a bounded timeout.
pub(crate) struct Session {
    inner: async_imap::Session<BoxedStream>,
    body_guard: Option<body::ReceiveGuard>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MailboxInfo {
    pub uid_validity: u32,
    pub uid_next: Option<u32>,
    pub exists: u32,
}

#[derive(Clone)]
pub(crate) struct RemoteHeader {
    pub uid: u32,
    pub flags: Vec<String>,
    pub internal_date: Option<DateTime<FixedOffset>>,
    pub size: Option<u32>,
    pub header: Vec<u8>,
}

/// Connects only along the account route. Credentials are sent after TLS.
pub(crate) async fn open_session(
    route: &NetRoute,
    login: &ImapLogin,
) -> Result<Session, ImapError> {
    let stream = net::connect(route, &login.host, login.port).await?;
    let client = match login.tls {
        TlsMode::Implicit => greet(net::upgrade_tls(stream, &login.host).await?).await?,
        TlsMode::StartTls => {
            let mut client = greet(stream).await?;
            step(client.run_command_and_check_ok("STARTTLS", None))
                .await
                .map_err(|error| match error {
                    ImapError::Timeout => error,
                    _ => ImapError::Unsupported("STARTTLS".into()),
                })?;
            // No greeting follows STARTTLS; the session continues inside TLS.
            Client::new(net::upgrade_tls(client.into_inner(), &login.host).await?)
        }
    };
    authenticate(client, login).await
}

/// The production connection entry remains `open_session`; tests can supply an
/// in-memory peer while exercising the same greeting and authentication flow.
#[cfg(test)]
pub(crate) async fn test_session(
    stream: BoxedStream,
    login: &ImapLogin,
) -> Result<Session, ImapError> {
    authenticate(greet(stream).await?, login).await
}

/// Compatibility wrapper for callers that only need a folder snapshot.
#[expect(
    dead_code,
    reason = "Retained folder-only entry point; workers share sessions across folders."
)]
pub(crate) async fn fetch_folders(
    route: &NetRoute,
    login: &ImapLogin,
) -> Result<Vec<RemoteFolder>, ImapError> {
    let mut session = open_session(route, login).await?;
    let folders = session.list_folders().await;
    let _ = session.logout().await;
    folders
}

/// Reads the server greeting that opens every IMAP connection.
async fn greet(stream: BoxedStream) -> Result<Client<BoxedStream>, ImapError> {
    let mut client = Client::new(stream);
    let greeting = step(async {
        client
            .read_response()
            .await
            .map_err(|_| ImapError::Protocol("failed to read IMAP greeting".into()))
    })
    .await?;
    match greeting {
        Some(_) => Ok(client),
        None => Err(ImapError::Protocol(
            "connection closed before the greeting".into(),
        )),
    }
}

async fn authenticate(
    client: Client<BoxedStream>,
    login: &ImapLogin,
) -> Result<Session, ImapError> {
    let mut inner = timeout(
        COMMAND_TIMEOUT,
        client.login(&login.username, login.password.expose_secret()),
    )
    .await
    .map_err(|_| ImapError::Timeout)?
    .map_err(|(error, _)| match error {
        ImapProtocolError::No(_) | ImapProtocolError::Bad(_) => {
            ImapError::Authentication("credentials were rejected".into())
        }
        other => ImapError::from(other),
    })?;

    let capabilities = step(inner.capabilities()).await?;
    let mut session = Session {
        inner,
        body_guard: None,
    };
    if capabilities.has_str("ID") {
        // Identification is optional, but timeout/EOF cannot leave a partially
        // consumed response on the connection used for mailbox commands.
        let command = format!(
            "ID (\"name\" \"MailX\" \"version\" \"{}\")",
            env!("CARGO_PKG_VERSION")
        );
        match session
            .checked_command::<()>(&command, |_, _| Ok(None))
            .await
        {
            Ok(_) | Err(ImapError::Rejected) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(session)
}

impl Session {
    pub(crate) async fn list_folders(&mut self) -> Result<Vec<RemoteFolder>, ImapError> {
        // async-imap 0.12's LIST stream drops tagged NO/BAD. Wait for an
        // explicit OK ourselves before allowing folder deletion reconciliation.
        let folders = self
            .checked_command("LIST \"\" *", |response, raw| {
                Ok(match response {
                    Response::MailboxData(MailboxDatum::List(name)) => Some(RemoteFolder {
                        name: mailbox_text(&name.name, raw),
                        delimiter: name
                            .delimiter
                            .as_ref()
                            .map(|delimiter| mailbox_text(delimiter, raw)),
                        attributes: name.name_attributes.iter().map(attribute_name).collect(),
                        status: None,
                    }),
                    _ => None,
                })
            })
            .await?;
        Ok(folders)
    }

    /// Call before EXAMINE: STATUS is not intended for the selected mailbox.
    pub(crate) async fn status(&mut self, name: &str) -> Result<FolderStatus, ImapError> {
        let command = format!(
            "STATUS {} (MESSAGES UNSEEN UIDNEXT UIDVALIDITY)",
            quoted_mailbox(name)?
        );
        let statuses = self
            .checked_command(&command, |response, raw| {
                let Response::MailboxData(MailboxDatum::Status { mailbox, status }) = response
                else {
                    return Ok(None);
                };
                let mailbox = mailbox_text(mailbox, raw);
                if mailbox != name
                    && !(mailbox.eq_ignore_ascii_case("INBOX")
                        && name.eq_ignore_ascii_case("INBOX"))
                {
                    return Ok(None);
                }
                let mut result = FolderStatus {
                    messages: 0,
                    unseen: 0,
                    uid_next: None,
                    uid_validity: None,
                };
                let mut has_messages = false;
                let mut has_unseen = false;
                for attribute in status {
                    match attribute {
                        StatusAttribute::Messages(value) => {
                            result.messages = *value;
                            has_messages = true;
                        }
                        StatusAttribute::Unseen(value) => {
                            result.unseen = *value;
                            has_unseen = true;
                        }
                        StatusAttribute::UidNext(value) => result.uid_next = Some(*value),
                        StatusAttribute::UidValidity(value) => result.uid_validity = Some(*value),
                        _ => {}
                    }
                }
                if !has_messages || !has_unseen {
                    return Err(ImapError::Protocol(
                        "STATUS omitted requested counts".into(),
                    ));
                }
                Ok(Some(result))
            })
            .await?;
        statuses
            .into_iter()
            .next()
            .ok_or_else(|| ImapError::Mailbox("STATUS did not return mailbox data".into()))
    }

    pub(crate) async fn examine(&mut self, name: &str) -> Result<MailboxInfo, ImapError> {
        let command = format!("EXAMINE {}", quoted_mailbox(name)?);
        let mut result = MailboxInfo {
            uid_validity: 0,
            uid_next: None,
            exists: 0,
        };
        self.checked_command::<()>(&command, |response, _| {
            match response {
                Response::MailboxData(MailboxDatum::Exists(value)) => result.exists = *value,
                Response::Data {
                    status: Status::Ok,
                    outcome,
                } => match outcome.code {
                    Some(ResponseCode::UidValidity(value)) => result.uid_validity = value,
                    Some(ResponseCode::UidNext(value)) => result.uid_next = Some(value),
                    _ => {}
                },
                _ => {}
            }
            Ok(None)
        })
        .await?;
        if result.uid_validity == 0 {
            return Err(ImapError::Mailbox(
                "EXAMINE did not return a valid UIDVALIDITY".into(),
            ));
        }
        Ok(result)
    }

    pub(crate) async fn search_since(&mut self, date: NaiveDate) -> Result<Vec<u32>, ImapError> {
        // Chrono's default %b is locale-independent English. No external input
        // may be interpolated into async-imap's unescaped SEARCH expression.
        let query = format!("UID SEARCH SINCE {}", date.format("%d-%b-%Y"));
        let batches = self
            .checked_command(&query, |response, _| {
                Ok(match response {
                    Response::MailboxData(MailboxDatum::Search(uids)) => Some(uids.clone()),
                    _ => None,
                })
            })
            .await?;
        if batches.is_empty() {
            return Err(ImapError::Mailbox(
                "SEARCH did not return enumeration data".into(),
            ));
        }
        let mut uids: Vec<_> = batches.into_iter().flatten().collect();
        validate_uids(&uids)?;
        uids.sort_unstable();
        uids.dedup();
        Ok(uids)
    }

    pub(crate) async fn fetch_headers(
        &mut self,
        uids: &[u32],
    ) -> Result<Vec<RemoteHeader>, ImapError> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        validate_uids(uids)?;
        if uids.len() > 200 {
            return Err(ImapError::Protocol("header batch exceeds 200 UIDs".into()));
        }
        // BODYSTRUCTURE is deliberately not requested: some servers (2925) write
        // Content-Disposition types as bare atoms, which the response parser
        // rejects, and one such message would fail every retry of its batch.
        // Attachments are inferred from CONTENT-TYPE / CONTENT-DISPOSITION.
        let query = format!(
            "UID FETCH {} (UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[HEADER.FIELDS (DATE SUBJECT FROM TO CC REPLY-TO MESSAGE-ID CONTENT-TYPE CONTENT-DISPOSITION)])",
            uid_set(uids)
        );
        self.checked_command(&query, |response, _| {
            let Response::Fetch(_, attributes) = response else {
                return Ok(None);
            };
            if flags_notification(attributes) {
                // Even a requested UID can receive an unsolicited flags-only
                // update. The driver verifies that all requested headers arrive.
                return Ok(None);
            }
            let Some((uid, flags)) = fetch_identity(attributes, uids)? else {
                return Ok(None);
            };
            let mut result = RemoteHeader {
                uid,
                flags,
                internal_date: None,
                size: None,
                header: Vec::new(),
            };
            let mut found_header = false;
            for attribute in attributes {
                match attribute {
                    AttributeValue::InternalDate(value) => {
                        result.internal_date =
                            DateTime::parse_from_str(value, "%d-%b-%Y %H:%M:%S %z").ok();
                    }
                    AttributeValue::Rfc822Size(value) => result.size = Some(*value),
                    AttributeValue::BodySection {
                        section: Some(SectionPath::Full(MessageSection::Header)),
                        data: Some(value),
                        ..
                    } => {
                        result.header = value.to_vec();
                        found_header = true;
                    }
                    _ => {}
                }
            }
            if !found_header {
                return Err(ImapError::Protocol(
                    "header response did not contain header data".into(),
                ));
            }
            Ok(Some(result))
        })
        .await
    }

    pub(crate) async fn fetch_flags(
        &mut self,
        uids: &[u32],
    ) -> Result<Vec<(u32, Vec<String>)>, ImapError> {
        validate_uids(uids)?;
        let mut flags = Vec::new();
        for chunk in uids.chunks(500) {
            let query = format!("UID FETCH {} (UID FLAGS)", uid_set(chunk));
            let fetched = self
                .checked_command(&query, |response, _| match response {
                    Response::Fetch(_, attributes) => fetch_identity(attributes, chunk),
                    _ => Ok(None),
                })
                .await?;
            flags.extend(fetched);
        }
        Ok(flags)
    }

    /// The library's FETCH, SEARCH and LIST collectors ignore tagged NO/BAD;
    /// STATUS and EXAMINE also accept EOF after partial data. Retain completion
    /// status ourselves so no partial response can look like a full snapshot.
    async fn checked_command<T>(
        &mut self,
        command: &str,
        mut collect: impl FnMut(&Response<'_>, &[u8]) -> Result<Option<T>, ImapError>,
    ) -> Result<Vec<T>, ImapError> {
        let result = step(async {
            let tag = self.inner.run_command(command).await?;
            let mut results = Vec::new();
            let mut collection_failed = false;
            loop {
                let response = self
                    .inner
                    .read_response()
                    .await
                    .map_err(|error| read_failure(command, &error))?
                    .ok_or_else(|| {
                        ImapError::Protocol("IMAP connection closed before completion".into())
                    })?;
                let raw = response.borrow_owner();
                match response.parsed() {
                    Response::Done {
                        tag: response_tag,
                        status,
                        ..
                    } if response_tag == &tag => {
                        return match status {
                            Status::Ok if collection_failed => Err(ImapError::Mailbox(
                                "command returned invalid mailbox data".into(),
                            )),
                            Status::Ok => Ok(results),
                            Status::No | Status::Bad => Err(ImapError::Rejected),
                            _ => Err(ImapError::Protocol("unexpected completion status".into())),
                        };
                    }
                    Response::Data {
                        status: Status::Bye,
                        ..
                    } => {
                        return Err(ImapError::Protocol(
                            "IMAP connection closed before completion".into(),
                        ));
                    }
                    Response::Done { .. } => {
                        return Err(ImapError::Protocol(
                            "unexpected command completion tag".into(),
                        ));
                    }
                    response => {
                        // Drain to the tagged completion even if one parsed row is
                        // invalid, so a healthy connection can serve later folders.
                        match collect(response, raw) {
                            Ok(Some(result)) => results.push(result),
                            Ok(None) => {}
                            Err(_) => collection_failed = true,
                        }
                    }
                }
            }
        })
        .await;
        match &self.body_guard {
            Some(guard) => guard.check(result),
            None => result,
        }
    }

    pub(crate) async fn logout(mut self) -> Result<(), ImapError> {
        step(self.inner.logout()).await
    }
}

/// imap-proto 0.17 retains escapes in quoted strings. Its borrowed mailbox
/// slice still points into the parsed response, so identify quoted versus
/// literal representation without reparsing IMAP or altering literal bytes.
fn mailbox_text(name: &str, response: &[u8]) -> String {
    let offset = (name.as_ptr() as usize).checked_sub(response.as_ptr() as usize);
    let quoted = offset.is_some_and(|offset| {
        offset > 0
            && response.get(offset - 1) == Some(&b'"')
            && response.get(offset + name.len()) == Some(&b'"')
    });
    if !quoted {
        return name.to_owned();
    }
    let mut chars = name.chars();
    let mut decoded = String::with_capacity(name.len());
    while let Some(character) = chars.next() {
        if character == '\\' {
            if let Some(escaped) = chars.next() {
                decoded.push(escaped);
            }
        } else {
            decoded.push(character);
        }
    }
    decoded
}

fn quoted_mailbox(name: &str) -> Result<String, ImapError> {
    if name.contains(['\r', '\n', '\0']) {
        return Err(ImapError::Protocol("invalid mailbox name".into()));
    }
    Ok(format!(
        "\"{}\"",
        name.replace('\\', "\\\\").replace('\"', "\\\"")
    ))
}

fn fetch_identity(
    attributes: &[AttributeValue<'_>],
    requested: &[u32],
) -> Result<Option<(u32, Vec<String>)>, ImapError> {
    let uid = attributes.iter().find_map(|attribute| match attribute {
        AttributeValue::Uid(uid) => Some(*uid),
        _ => None,
    });
    let uid = match uid {
        Some(uid) if requested.contains(&uid) => uid,
        Some(_) => return Ok(None),
        None if flags_notification(attributes) => return Ok(None),
        None => {
            return Err(ImapError::Protocol(
                "FETCH response did not contain a requested UID".into(),
            ));
        }
    };
    let flags = attributes
        .iter()
        .find_map(|attribute| match attribute {
            AttributeValue::Flags(flags) => Some(
                flags
                    .iter()
                    .map(|flag| flag_name(Flag::from(flag.as_ref())))
                    .collect(),
            ),
            _ => None,
        })
        .ok_or_else(|| ImapError::Protocol("FETCH response did not contain flags".into()))?;
    Ok(Some((uid, flags)))
}

fn flags_notification(attributes: &[AttributeValue<'_>]) -> bool {
    attributes
        .iter()
        .any(|attribute| matches!(attribute, AttributeValue::Flags(_)))
        && attributes.iter().all(|attribute| {
            matches!(
                attribute,
                AttributeValue::Uid(_) | AttributeValue::Flags(_) | AttributeValue::ModSeq(_)
            )
        })
}

fn validate_uids(uids: &[u32]) -> Result<(), ImapError> {
    if uids.contains(&0) {
        Err(ImapError::Protocol("UID must be nonzero".into()))
    } else {
        Ok(())
    }
}

fn uid_set(uids: &[u32]) -> String {
    let mut uids = uids.to_vec();
    uids.sort_unstable();
    uids.dedup();
    let mut ranges = Vec::new();
    let mut remaining = uids.as_slice();
    while let Some((&start, tail)) = remaining.split_first() {
        let mut end = start;
        remaining = tail;
        while let Some((&next, tail)) = remaining.split_first() {
            if end.checked_add(1) != Some(next) {
                break;
            }
            end = next;
            remaining = tail;
        }
        ranges.push(if start == end {
            start.to_string()
        } else {
            format!("{start}:{end}")
        });
    }
    ranges.join(",")
}

fn flag_name(flag: Flag<'_>) -> String {
    match flag {
        Flag::Seen => "\\Seen".into(),
        Flag::Answered => "\\Answered".into(),
        Flag::Flagged => "\\Flagged".into(),
        Flag::Deleted => "\\Deleted".into(),
        Flag::Draft => "\\Draft".into(),
        Flag::Recent => "\\Recent".into(),
        Flag::MayCreate => "\\*".into(),
        Flag::Custom(value) => {
            // async-imap recognizes system flags case-sensitively. Normalize
            // noncanonical spellings while preserving all custom keywords.
            [
                "\\Seen",
                "\\Answered",
                "\\Flagged",
                "\\Deleted",
                "\\Draft",
                "\\Recent",
            ]
            .into_iter()
            .find(|flag| flag.eq_ignore_ascii_case(&value))
            .map_or_else(|| value.into_owned(), str::to_owned)
        }
    }
}

fn attribute_name(attribute: &NameAttribute<'_>) -> String {
    match attribute {
        NameAttribute::NoInferiors => "\\Noinferiors".into(),
        NameAttribute::NoSelect => "\\Noselect".into(),
        NameAttribute::Marked => "\\Marked".into(),
        NameAttribute::Unmarked => "\\Unmarked".into(),
        NameAttribute::All => "\\All".into(),
        NameAttribute::Archive => "\\Archive".into(),
        NameAttribute::Drafts => "\\Drafts".into(),
        NameAttribute::Flagged => "\\Flagged".into(),
        NameAttribute::Junk => "\\Junk".into(),
        NameAttribute::Sent => "\\Sent".into(),
        NameAttribute::Trash => "\\Trash".into(),
        NameAttribute::Extension(name) => name.to_string(),
        other => format!("{other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};

    use super::*;

    #[test]
    fn read_failures_name_the_command_and_cause_without_server_text() {
        use std::io::{Error as IoError, ErrorKind};
        let secret = "SUBJECT leaked-secret";
        let cases = [
            (
                IoError::other(format!("Error(..) during parsing of {secret}")),
                "UID FETCH: server response could not be parsed",
            ),
            (
                IoError::new(ErrorKind::UnexpectedEof, secret),
                "UID FETCH: connection ended in the middle of a response",
            ),
            (
                IoError::new(ErrorKind::ConnectionReset, secret),
                "UID FETCH: connection was reset by the server or network",
            ),
            (
                IoError::new(ErrorKind::TimedOut, secret),
                "UID FETCH: network error while reading response",
            ),
        ];
        for (error, expected) in cases {
            let ImapError::Protocol(text) = read_failure("UID FETCH 1:5 (UID FLAGS)", &error)
            else {
                panic!("not a protocol error");
            };
            assert_eq!(text, expected);
            assert!(!text.contains("leaked-secret"));
        }
        let ImapError::Protocol(text) = read_failure(
            "STATUS \"Inbox\" (MESSAGES)",
            &IoError::new(ErrorKind::BrokenPipe, "x"),
        ) else {
            panic!()
        };
        assert_eq!(
            text,
            "STATUS: connection was reset by the server or network"
        );
    }

    fn login() -> ImapLogin {
        ImapLogin {
            host: "imap.example.com".into(),
            port: 993,
            tls: TlsMode::Implicit,
            username: "lin@example.com".into(),
            password: SecretValue::new("app \"pass\"".into()),
        }
    }

    /// Plays the server side of a session: for each expected command prefix,
    /// checks the client line and answers with `reply`, where `{tag}` is the
    /// client's tag.
    async fn serve(server: DuplexStream, script: Vec<(String, String)>) {
        let mut server = BufReader::new(server);
        server
            .get_mut()
            .write_all(b"* OK IMAP4rev1 ready\r\n")
            .await
            .unwrap();
        for (expected, reply) in script {
            let mut line = String::new();
            server.read_line(&mut line).await.unwrap();
            let (tag, command) = line.trim_end().split_once(' ').unwrap();
            assert!(
                command.starts_with(&expected),
                "expected {expected:?}, got {command:?}"
            );
            assert!(!command.starts_with("SELECT"));
            assert!(!command.starts_with("STORE") && !command.starts_with("UID STORE"));
            assert!(!command.contains("BODY["));
            server
                .get_mut()
                .write_all(reply.replace("{tag}", tag).as_bytes())
                .await
                .unwrap();
        }
    }

    async fn run(
        script: Vec<(&'static str, &'static str)>,
    ) -> Result<Vec<RemoteFolder>, ImapError> {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let server = tokio::spawn(serve(
            server,
            script
                .into_iter()
                .map(|(a, b)| (a.into(), b.into()))
                .collect(),
        ));
        let result = async {
            let mut session = authenticate(greet(Box::new(client)).await?, &login()).await?;
            let folders = session.list_folders().await?;
            let _ = session.logout().await;
            Ok(folders)
        }
        .await;
        server.await.unwrap();
        result
    }

    #[tokio::test]
    async fn lists_folders_without_coupling_discovery_to_status() {
        let folders = run(vec![
            (
                r#"LOGIN "lin@example.com" "app \"pass\"""#,
                "{tag} OK LOGIN completed\r\n",
            ),
            (
                "CAPABILITY",
                "* CAPABILITY IMAP4rev1 ID SPECIAL-USE\r\n{tag} OK done\r\n",
            ),
            (
                "ID (\"name\" \"MailX\"",
                "* ID NIL\r\n{tag} OK ID completed\r\n",
            ),
            (
                "LIST \"\" *",
                "* LIST (\\HasNoChildren) \"/\" INBOX\r\n\
                 * LIST (\\HasNoChildren \\Sent) \"/\" \"&XfJT0ZAB-\"\r\n\
                 * LIST (\\Noselect \\HasChildren) \"/\" \"Clients\"\r\n\
                 * LIST (\\HasNoChildren) \"/\" \"Clients/East\"\r\n\
                 {tag} OK LIST completed\r\n",
            ),
            ("LOGOUT", "* BYE\r\n{tag} OK LOGOUT completed\r\n"),
        ])
        .await
        .unwrap();

        let names: Vec<_> = folders.iter().map(|folder| folder.name.as_str()).collect();
        assert_eq!(names, ["INBOX", "&XfJT0ZAB-", "Clients", "Clients/East"]);
        assert!(folders.iter().all(|folder| folder.status.is_none()));
        assert!(
            folders[1]
                .attributes
                .iter()
                .any(|attribute| attribute == "\\Sent")
        );
        assert_eq!(
            folders[2].status, None,
            "\\Noselect folders are not queried"
        );
        assert_eq!(folders[3].delimiter.as_deref(), Some("/"));
    }

    #[tokio::test]
    async fn rejected_login_is_an_authentication_error() {
        let error = run(vec![(
            "LOGIN",
            "{tag} NO [AUTHENTICATIONFAILED] Invalid credentials\r\n",
        )])
        .await
        .unwrap_err();
        assert!(
            matches!(error, ImapError::Authentication(reason) if reason == "credentials were rejected")
        );
    }

    #[tokio::test]
    async fn skips_id_when_the_server_does_not_offer_it() {
        let folders = run(vec![
            ("LOGIN", "{tag} OK\r\n"),
            ("CAPABILITY", "* CAPABILITY IMAP4rev1\r\n{tag} OK\r\n"),
            ("LIST", "* LIST () \".\" INBOX\r\n{tag} OK\r\n"),
            ("LOGOUT", "{tag} OK\r\n"),
        ])
        .await
        .unwrap();
        assert_eq!(folders.len(), 1);
    }

    #[tokio::test]
    async fn optional_id_refusal_is_safe_but_connection_loss_is_not() {
        let login = ("LOGIN", "{tag} OK\r\n");
        let capabilities = ("CAPABILITY", "* CAPABILITY IMAP4rev1 ID\r\n{tag} OK\r\n");
        let folders = run(vec![
            login,
            capabilities,
            ("ID", "{tag} NO optional ID unavailable\r\n"),
            ("LIST", "* LIST () \"/\" INBOX\r\n{tag} OK\r\n"),
            ("LOGOUT", "{tag} OK\r\n"),
        ])
        .await
        .unwrap();
        assert_eq!(folders.len(), 1);
        let error = run(vec![login, capabilities, ("ID", "* BYE disconnected\r\n")])
            .await
            .unwrap_err();
        assert!(!error.session_usable());
    }

    async fn header_session(
        commands: Vec<(String, String)>,
    ) -> (Session, tokio::task::JoinHandle<()>) {
        let (client, server) = tokio::io::duplex(64 * 1024);
        let mut script = vec![
            ("LOGIN".into(), "{tag} OK\r\n".into()),
            (
                "CAPABILITY".into(),
                "* CAPABILITY IMAP4rev1\r\n{tag} OK\r\n".into(),
            ),
        ];
        script.extend(commands);
        let server = tokio::spawn(serve(server, script));
        let session = test_session(Box::new(client), &login()).await.unwrap();
        (session, server)
    }

    #[test]
    fn formats_uid_sets_without_duplicates_or_overflow() {
        assert_eq!(uid_set(&[]), "");
        assert_eq!(uid_set(&[9]), "9");
        assert_eq!(uid_set(&[12, 1, 2, 4, 3, 5, 7, 9, 10, 11, 2]), "1:5,7,9:12");
        assert_eq!(uid_set(&[u32::MAX, u32::MAX - 1]), "4294967294:4294967295");
    }

    #[test]
    fn normalizes_system_flags_without_losing_keywords() {
        assert_eq!(flag_name(Flag::from("\\sEeN")), "\\Seen");
        assert_eq!(flag_name(Flag::from("\\FLAGGED")), "\\Flagged");
        assert_eq!(flag_name(Flag::from("$Forwarded")), "$Forwarded");
    }

    #[tokio::test]
    async fn reads_headers_through_a_read_only_session() {
        let header = b"Date: Fri, 18 Sep 2026 10:00:00 +0800\r\nSubject: hello\r\nFrom: Lin <lin@example.com>\r\n\r\n";
        let response = format!(
            "* 1 FETCH (UID 42 FLAGS (\\seen \\Flagged $Forwarded) INTERNALDATE \"18-Sep-2026 10:00:00 +0800\" RFC822.SIZE 2048 BODY[HEADER.FIELDS (DATE SUBJECT FROM TO CC REPLY-TO MESSAGE-ID CONTENT-TYPE CONTENT-DISPOSITION)] {{{}}}\r\n{})\r\n{{tag}} OK fetched\r\n",
            header.len(),
            String::from_utf8_lossy(header),
        );
        let (mut session, server) = header_session(vec![
            ("STATUS \"INBOX\" (MESSAGES UNSEEN UIDNEXT UIDVALIDITY)".into(), "* STATUS INBOX (MESSAGES 3 UNSEEN 2 UIDNEXT 43 UIDVALIDITY 9)\r\n{tag} OK\r\n".into()),
            ("EXAMINE \"INBOX\"".into(), "* 3 EXISTS\r\n* OK [UIDVALIDITY 9] valid\r\n* OK [UIDNEXT 43] next\r\n{tag} OK [READ-ONLY]\r\n".into()),
            ("UID SEARCH SINCE 18-Sep-2026".into(), "* SEARCH 42 7 19\r\n{tag} OK\r\n".into()),
            ("UID FETCH 42 (UID FLAGS INTERNALDATE RFC822.SIZE BODY.PEEK[HEADER.FIELDS (DATE SUBJECT FROM TO CC REPLY-TO MESSAGE-ID CONTENT-TYPE CONTENT-DISPOSITION)])".into(), response),
            ("UID FETCH 42 (UID FLAGS)".into(), "* 1 FETCH (UID 42 FLAGS (\\Seen))\r\n{tag} OK\r\n".into()),
            ("LOGOUT".into(), "{tag} OK\r\n".into()),
        ]).await;
        assert_eq!(session.status("INBOX").await.unwrap().unseen, 2);
        assert_eq!(
            session.examine("INBOX").await.unwrap(),
            MailboxInfo {
                uid_validity: 9,
                uid_next: Some(43),
                exists: 3
            }
        );
        assert_eq!(
            session
                .search_since(NaiveDate::from_ymd_opt(2026, 9, 18).unwrap())
                .await
                .unwrap(),
            vec![7, 19, 42]
        );
        let fetched = session.fetch_headers(&[42]).await.unwrap();
        assert_eq!(fetched.len(), 1);
        let fetched = &fetched[0];
        assert_eq!(fetched.uid, 42);
        assert_eq!(fetched.header, header);
        assert_eq!(fetched.flags, ["\\Seen", "\\Flagged", "$Forwarded"]);
        assert_eq!(
            fetched.internal_date.unwrap().to_rfc3339(),
            "2026-09-18T10:00:00+08:00"
        );
        assert_eq!(fetched.size, Some(2048));
        assert_eq!(
            session.fetch_flags(&[42]).await.unwrap(),
            vec![(42, vec!["\\Seen".into()])]
        );
        session.logout().await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn requires_uid_validity_before_sync() {
        let (mut session, server) = header_session(vec![(
            "EXAMINE".into(),
            "* 1 EXISTS\r\n{tag} OK [READ-ONLY]\r\n".into(),
        )])
        .await;
        assert!(matches!(
            session.examine("INBOX").await,
            Err(ImapError::Mailbox(_))
        ));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn empty_or_invalid_uid_requests_send_no_fetch() {
        let (mut session, server) =
            header_session(vec![("LOGOUT".into(), "{tag} OK\r\n".into())]).await;
        assert!(session.fetch_headers(&[]).await.unwrap().is_empty());
        assert!(session.fetch_flags(&[]).await.unwrap().is_empty());
        assert!(session.fetch_headers(&[0]).await.is_err());
        assert!(session.fetch_flags(&[0]).await.is_err());
        assert!(
            session
                .fetch_headers(&(1..=201).collect::<Vec<_>>())
                .await
                .is_err()
        );
        session.logout().await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn flags_are_split_into_commands_of_at_most_500_uids() {
        let (mut session, server) = header_session(vec![
            (
                "UID FETCH 1:500 (UID FLAGS)".into(),
                "* 1 FETCH (UID 1 FLAGS (\\Seen))\r\n{tag} OK\r\n".into(),
            ),
            (
                "UID FETCH 501 (UID FLAGS)".into(),
                "* 2 FETCH (UID 501 FLAGS (\\Flagged))\r\n{tag} OK\r\n".into(),
            ),
        ])
        .await;
        let flags = session
            .fetch_flags(&(1..=501).collect::<Vec<_>>())
            .await
            .unwrap();
        assert_eq!(
            flags,
            vec![(1, vec!["\\Seen".into()]), (501, vec!["\\Flagged".into()])]
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn failed_fetch_discards_partial_results_and_redacts_server_text() {
        let (mut session, server) = header_session(vec![(
            "UID FETCH".into(),
            "* 1 FETCH (UID 1 FLAGS (\\Seen))\r\n{tag} NO private subject and password\r\n".into(),
        )])
        .await;
        let error = session.fetch_flags(&[1, 2]).await.unwrap_err();
        assert_eq!(error.to_string(), "protocol error: IMAP command failed");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn search_requires_an_explicit_enumeration_even_when_empty() {
        let date = NaiveDate::from_ymd_opt(2026, 9, 18).unwrap();
        let (mut session, server) = header_session(vec![
            ("UID SEARCH".into(), "{tag} OK\r\n".into()),
            ("UID SEARCH".into(), "* SEARCH\r\n{tag} OK\r\n".into()),
        ])
        .await;
        assert!(session.search_since(date).await.is_err());
        assert!(session.search_since(date).await.unwrap().is_empty());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn fetch_ignores_unsolicited_flags_and_unrequested_uids() {
        let notifications =
            "* 9 FETCH (FLAGS (\\Seen))\r\n* 8 FETCH (UID 99 FLAGS (\\Flagged))\r\n";
        let header = "Subject: hello\r\n\r\n";
        let headers_reply = format!(
            "{notifications}* 1 FETCH (UID 42 FLAGS (\\Seen))\r\n* 1 FETCH (UID 42 FLAGS () BODY[HEADER.FIELDS (SUBJECT)] {{{}}}\r\n{header})\r\n{{tag}} OK\r\n",
            header.len()
        );
        let (mut session, server) = header_session(vec![
            ("UID FETCH".into(), headers_reply),
            (
                "UID FETCH".into(),
                format!("{notifications}* 1 FETCH (UID 42 FLAGS (\\Seen))\r\n{{tag}} OK\r\n"),
            ),
            (
                "UID FETCH".into(),
                "* 1 FETCH (UID 42 FLAGS () RFC822.SIZE 100)\r\n{tag} OK\r\n".into(),
            ),
        ])
        .await;
        let headers = session.fetch_headers(&[42]).await.unwrap();
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].uid, 42);
        assert_eq!(headers[0].header, header.as_bytes());
        assert_eq!(
            session.fetch_flags(&[42]).await.unwrap(),
            vec![(42, vec!["\\Seen".into()])]
        );
        assert!(session.fetch_headers(&[42]).await.is_err());
        server.await.unwrap();
    }
    #[tokio::test]
    async fn failed_search_discards_partial_enumeration() {
        let (mut session, server) = header_session(vec![(
            "UID SEARCH".into(),
            "* SEARCH 1 2\r\n{tag} NO private mailbox\r\n".into(),
        )])
        .await;
        assert!(matches!(
            session
                .search_since(NaiveDate::from_ymd_opt(2026, 9, 18).unwrap())
                .await,
            Err(ImapError::Rejected)
        ));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn failed_list_cannot_be_mistaken_for_a_folder_snapshot() {
        let result = run(vec![
            ("LOGIN", "{tag} OK\r\n"),
            ("CAPABILITY", "* CAPABILITY IMAP4rev1\r\n{tag} OK\r\n"),
            (
                "LIST",
                "* LIST () \"/\" INBOX\r\n{tag} BAD private mailbox\r\n",
            ),
        ])
        .await;
        assert!(matches!(result, Err(ImapError::Rejected)));
    }

    #[tokio::test]
    async fn status_and_examine_reject_failed_or_incomplete_responses() {
        for (command, reply) in [
            (
                "STATUS",
                "* STATUS INBOX (MESSAGES 3 UNSEEN 2)\r\n{tag} NO private mailbox\r\n",
            ),
            (
                "EXAMINE",
                "* OK [UIDVALIDITY 9] valid\r\n{tag} BAD private mailbox\r\n",
            ),
            ("STATUS", "* STATUS INBOX (MESSAGES 3 UNSEEN 2)\r\n"),
            ("EXAMINE", "* OK [UIDVALIDITY 9] valid\r\n"),
        ] {
            let (mut session, server) = header_session(vec![(command.into(), reply.into())]).await;
            let failed = if command == "STATUS" {
                session.status("INBOX").await.is_err()
            } else {
                session.examine("INBOX").await.is_err()
            };
            server.await.unwrap();
            assert!(failed, "accepted incomplete or failed {command}");
        }
    }

    #[tokio::test]
    async fn checked_fetch_requires_completion_even_after_rows_arrive() {
        let (mut session, server) = header_session(vec![(
            "UID FETCH".into(),
            "* 1 FETCH (UID 1 FLAGS (\\Seen))\r\n".into(),
        )])
        .await;
        assert!(session.fetch_flags(&[1]).await.is_err());
        server.await.unwrap();
    }
    #[test]
    fn protocol_errors_never_include_server_payloads() {
        let errors = [
            ImapProtocolError::Bad("secret header".into()),
            ImapProtocolError::No("secret password".into()),
            ImapProtocolError::Parse(async_imap::error::ParseError::Unexpected(
                "secret header".into(),
            )),
        ];
        for error in errors {
            assert_eq!(
                ImapError::from(error).to_string(),
                "protocol error: IMAP command failed"
            );
        }
    }

    #[tokio::test]
    async fn escaped_quoted_and_literal_mailbox_names_keep_distinct_identity() {
        let literal_name = "Literal \\\"team\\\"";
        let list = format!(
            "* LIST () \"/\" \"Quoted \\\"team\\\"\\\\box\"\r\n* LIST () \"/\" {{{}}}\r\n{}\r\n{{tag}} OK\r\n",
            literal_name.len(),
            literal_name
        );
        let (mut session, server) = header_session(vec![
            ("LIST".into(), list),
            (
                "STATUS \"Quoted \\\"team\\\"\\\\box\"".into(),
                "* STATUS \"Quoted \\\"team\\\"\\\\box\" (MESSAGES 1 UNSEEN 1)\r\n{tag} OK\r\n"
                    .into(),
            ),
            (
                format!("STATUS {}", quoted_mailbox(literal_name).unwrap()),
                format!(
                    "* STATUS {{{}}}\r\n{} (MESSAGES 2 UNSEEN 1)\r\n{{tag}} OK\r\n",
                    literal_name.len(),
                    literal_name
                ),
            ),
            ("LOGOUT".into(), "{tag} OK\r\n".into()),
        ])
        .await;
        let listed = session.list_folders().await.unwrap();
        assert_eq!(listed[0].name, "Quoted \"team\"\\box");
        assert_eq!(listed[1].name, literal_name);
        assert_eq!(session.status(&listed[0].name).await.unwrap().messages, 1);
        assert_eq!(session.status(&listed[1].name).await.unwrap().messages, 2);
        session.logout().await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn malformed_completed_mailbox_data_is_drained_before_next_folder() {
        let (mut session, server) = header_session(vec![
            (
                "UID FETCH".into(),
                "* 1 FETCH (UID 1 RFC822.SIZE 20)\r\n{tag} OK\r\n".into(),
            ),
            (
                "STATUS".into(),
                "* STATUS Later (MESSAGES 2)\r\n{tag} OK\r\n".into(),
            ),
            (
                "EXAMINE".into(),
                "* OK [UIDVALIDITY 0] invalid\r\n{tag} OK\r\n".into(),
            ),
            (
                "EXAMINE".into(),
                "* OK [UIDVALIDITY 9] valid\r\n{tag} OK\r\n".into(),
            ),
            ("LOGOUT".into(), "{tag} OK\r\n".into()),
        ])
        .await;
        assert!(
            session
                .fetch_flags(&[1])
                .await
                .unwrap_err()
                .session_usable()
        );
        assert!(session.status("Later").await.unwrap_err().session_usable());
        assert!(session.examine("Later").await.unwrap_err().session_usable());
        assert_eq!(session.examine("Later").await.unwrap().uid_validity, 9);
        session.logout().await.unwrap();
        server.await.unwrap();
    }

    #[tokio::test]
    async fn eof_bye_and_unexpected_completion_tags_poison_the_connection() {
        for reply in ["* SEARCH 1\r\n", "* BYE private\r\n", "wrong OK\r\n"] {
            let (mut session, server) =
                header_session(vec![("UID SEARCH".into(), reply.into())]).await;
            let error = session
                .search_since(NaiveDate::from_ymd_opt(2026, 9, 18).unwrap())
                .await
                .unwrap_err();
            assert!(!error.session_usable());
            server.await.unwrap();
        }
    }
}
