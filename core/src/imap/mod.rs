//! Read-only IMAP operations for folder/header synchronization and body loading.

mod body;
mod policy;
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
    AttributeValue, Capability, MailboxDatum, MessageSection, Response, ResponseCode, SectionPath,
    Status, StatusAttribute,
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
    /// A recognized provider security policy rejected access. Never contains server text.
    ProviderPolicy,
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
            Self::ProviderPolicy => formatter.write_str("provider security policy rejected access"),
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
    fn from(error: ImapProtocolError) -> Self {
        if policy::library_rejection(&error) {
            return Self::ProviderPolicy;
        }
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
    /// The `[CODE]` of the tagged OK that ended the last checked command, e.g.
    /// `READ-ONLY` or `READ-WRITE`. Only SELECT reads it.
    completion: Option<ResponseCodeKind>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResponseCodeKind {
    ReadOnly,
    ReadWrite,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MailboxInfo {
    pub uid_validity: u32,
    pub uid_next: Option<u32>,
    /// Missing EXISTS must not be mistaken for an empty mailbox.
    pub exists: Option<u32>,
}

/// A folder opened for writing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WritableMailbox {
    pub uid_validity: u32,
    /// Flags the server will keep (`PERMANENTFLAGS`). `\\*` lets clients make up
    /// keywords but says nothing about `\\Flagged`, so it is kept as written.
    pub permanent_flags: Vec<String>,
    pub read_only: bool,
}

impl WritableMailbox {
    /// Whether `flag` (e.g. `\\Flagged`) can be stored here. A server that
    /// sends no `PERMANENTFLAGS` at all makes no promise, which reads as
    /// "unknown" rather than "no".
    pub(crate) fn keeps(&self, flag: &str) -> Option<bool> {
        if self.permanent_flags.is_empty() {
            return None;
        }
        Some(
            self.permanent_flags
                .iter()
                .any(|kept| kept.eq_ignore_ascii_case(flag)),
        )
    }
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
    let guard = body::ReceiveGuard::default();
    let result = async {
        let (client, gate) = connect_client(route, login, &guard).await?;
        let result = authenticate(client, login).await;
        // Header synchronization has its own command collectors. Keep this
        // receive gate only for greeting/login, whose library parser drops BYE.
        gate.disable();
        result
    }
    .await;
    guard.check(result)
}

/// Transport setup shared with the dedicated body connection. Every plaintext
/// authentication phase is bounded, including unsolicited policy rejections.
async fn connect_client(
    route: &NetRoute,
    login: &ImapLogin,
    guard: &body::ReceiveGuard,
) -> Result<(Client<BoxedStream>, body::PlaintextGate), ImapError> {
    let stream = net::connect(route, &login.host, login.port).await?;
    match login.tls {
        TlsMode::Implicit => {
            let stream = net::upgrade_tls(stream, &login.host).await?;
            let (stream, gate) = guard.wrap(stream);
            Ok((greet(stream).await?, gate))
        }
        TlsMode::StartTls => {
            let (stream, plaintext) = guard.wrap(stream);
            let mut client = greet(stream).await?;
            step(client.run_command_and_check_ok("STARTTLS", None))
                .await
                .map_err(|error| match error {
                    ImapError::Timeout | ImapError::ProviderPolicy => error,
                    _ => ImapError::Unsupported("STARTTLS".into()),
                })?;
            plaintext.disable();
            let stream = net::upgrade_tls(client.into_inner(), &login.host).await?;
            let (stream, gate) = guard.wrap(stream);
            // STARTTLS continues inside TLS without another greeting.
            Ok((Client::new(stream), gate))
        }
    }
}

/// The production connection entry remains `open_session`; tests can supply an
/// in-memory peer while exercising the same greeting and authentication flow.
#[cfg(test)]
pub(crate) async fn test_session(
    stream: BoxedStream,
    login: &ImapLogin,
) -> Result<Session, ImapError> {
    let guard = body::ReceiveGuard::default();
    let (stream, gate) = guard.wrap(stream);
    let result = async { authenticate(greet(stream).await?, login).await }.await;
    gate.disable();
    guard.check(result)
}

/// A session over an in-memory peer that may SELECT and STORE, for the write-back
/// tests in other modules. The read-only `test_session` peers forbid both.
#[cfg(test)]
pub(crate) async fn test_writable_session(
    commands: Vec<(String, String)>,
) -> (Session, tokio::task::JoinHandle<()>) {
    tests::writable_session(commands).await
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
        Some(response) if policy::response(response.parsed()) => Err(ImapError::ProviderPolicy),
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
    let inner = timeout(
        COMMAND_TIMEOUT,
        client.login(&login.username, login.password.expose_secret()),
    )
    .await
    .map_err(|_| ImapError::Timeout)?
    .map_err(|(error, _)| match error {
        error if policy::library_rejection(&error) => ImapError::ProviderPolicy,
        ImapProtocolError::No(_) | ImapProtocolError::Bad(_) => {
            ImapError::Authentication("credentials were rejected".into())
        }
        other => ImapError::from(other),
    })?;

    let mut session = Session {
        inner,
        body_guard: None,
        completion: None,
    };
    // The library collector drops tagged NO/BAD. Use the same checked
    // completion boundary as mailbox commands so policy refusals cannot vanish.
    let capabilities = session
        .checked_command("CAPABILITY", |response, _| {
            Ok(match response {
            Response::Capabilities(values) => Some(values.iter().any(|value|
                matches!(value, Capability::Atom(name) if name.eq_ignore_ascii_case("ID")))),
            _ => None,
        })
        })
        .await?;
    if capabilities.into_iter().any(|has_id| has_id) {
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
    /// Authentication diagnostics never select a mailbox or fetch message data.
    pub(crate) async fn verify_noop(&mut self) -> Result<(), ImapError> {
        self.checked_command::<()>("NOOP", |_, _| Ok(None)).await?;
        Ok(())
    }

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

    /// Some servers return lossy names such as `???` in STATUS while LIST and
    /// EXAMINE still accept the original modified UTF-7 name. Never guess which
    /// folder that row belongs to: read counts from the explicitly opened folder.
    /// Return its selection so a full sync can reuse EXAMINE.
    pub(crate) async fn status_or_examine(
        &mut self,
        name: &str,
    ) -> Result<(FolderStatus, Option<MailboxInfo>), ImapError> {
        match self.status(name).await {
            Ok(status) => return Ok((status, None)),
            // Only completed OK with unusable data is eligible. Rejections and
            // broken/incomplete connections retain their normal failure paths.
            Err(ImapError::Mailbox(_)) => {}
            Err(error) => return Err(error),
        }
        let mailbox = self.examine(name).await?;
        let messages = mailbox
            .exists
            .ok_or_else(|| ImapError::Mailbox("EXAMINE did not return a message count".into()))?;
        // This is the whole folder, not the fixed header window. EXAMINE's
        // [UNSEEN n] is a sequence number, not an unread count.
        let unseen = u32::try_from(self.search("UID SEARCH UNSEEN").await?.len())
            .map_err(|_| ImapError::Mailbox("unread count exceeds the supported range".into()))?;
        Ok((
            FolderStatus {
                messages,
                unseen,
                uid_next: mailbox.uid_next,
                uid_validity: Some(mailbox.uid_validity),
            },
            Some(mailbox),
        ))
    }

    pub(crate) async fn examine(&mut self, name: &str) -> Result<MailboxInfo, ImapError> {
        let command = format!("EXAMINE {}", quoted_mailbox(name)?);
        let mut result = MailboxInfo {
            uid_validity: 0,
            uid_next: None,
            exists: None,
        };
        self.checked_command::<()>(&command, |response, _| {
            match response {
                Response::MailboxData(MailboxDatum::Exists(value)) => result.exists = Some(*value),
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

    /// Opens a folder for writing. Only the flag write-back uses this, on its own
    /// connection: the header sync stays on read-only `EXAMINE`.
    pub(crate) async fn select(&mut self, name: &str) -> Result<WritableMailbox, ImapError> {
        let command = format!("SELECT {}", quoted_mailbox(name)?);
        let mut mailbox = WritableMailbox {
            uid_validity: 0,
            permanent_flags: Vec::new(),
            read_only: false,
        };
        self.checked_command::<()>(&command, |response, _| {
            if let Response::Data {
                status: Status::Ok,
                outcome,
            } = response
            {
                match outcome.code {
                    Some(ResponseCode::UidValidity(value)) => mailbox.uid_validity = value,
                    Some(ResponseCode::PermanentFlags(ref flags)) => {
                        mailbox.permanent_flags =
                            flags.iter().map(|flag| flag.to_string()).collect();
                    }
                    Some(ResponseCode::ReadOnly) => mailbox.read_only = true,
                    _ => {}
                }
            }
            Ok(None)
        })
        .await?;
        // The server states the mode in the tagged OK, not in an untagged line.
        mailbox.read_only |= self.completion == Some(ResponseCodeKind::ReadOnly);
        if mailbox.uid_validity == 0 {
            return Err(ImapError::Mailbox(
                "SELECT did not return a valid UIDVALIDITY".into(),
            ));
        }
        Ok(mailbox)
    }

    /// Sets or clears one flag on one message of the selected folder and returns
    /// the flags the server reports for it afterwards. `Rejected` (a tagged NO or
    /// BAD) is an answer, not a transport failure: the connection stays usable.
    pub(crate) async fn store_flag(
        &mut self,
        uid: u32,
        flag: &str,
        set: bool,
    ) -> Result<Vec<String>, ImapError> {
        validate_uids(&[uid])?;
        // Only the two flags this client manages are ever sent, and never built
        // from anything the server said.
        if !matches!(flag, "\\Seen" | "\\Flagged") {
            return Err(ImapError::Protocol("unsupported flag".into()));
        }
        let command = format!(
            "UID STORE {uid} {}FLAGS ({flag})",
            if set { "+" } else { "-" }
        );
        let results = self
            .checked_command(&command, |response, _| match response {
                Response::Fetch(_, attributes) => fetch_identity(attributes, &[uid]),
                _ => Ok(None),
            })
            .await?;
        // A server may stay silent when `.SILENT` semantics apply or the message
        // is gone; without the flags back the write cannot be confirmed.
        let flags = results
            .into_iter()
            .find(|(fetched, _)| *fetched == uid)
            .map(|(_, flags)| flags)
            .ok_or_else(|| ImapError::Mailbox("STORE was not confirmed".into()))?;
        let present = flags.iter().any(|kept| kept.eq_ignore_ascii_case(flag));
        if present != set {
            return Err(ImapError::Mailbox(
                "the server did not apply the flag".into(),
            ));
        }
        Ok(flags)
    }

    pub(crate) async fn search_since(&mut self, date: NaiveDate) -> Result<Vec<u32>, ImapError> {
        // Chrono's default %b is locale-independent English. No external input
        // may be interpolated into async-imap's unescaped SEARCH expression.
        let query = format!("UID SEARCH SINCE {}", date.format("%d-%b-%Y"));
        self.search(&query).await
    }

    async fn search(&mut self, query: &str) -> Result<Vec<u32>, ImapError> {
        let batches = self
            .checked_command(query, |response, _| {
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
                if policy::response(response.parsed()) {
                    return Err(ImapError::ProviderPolicy);
                }
                match response.parsed() {
                    Response::Done {
                        tag: response_tag,
                        status,
                        outcome,
                    } if response_tag == &tag => {
                        self.completion = Some(match outcome.code {
                            Some(ResponseCode::ReadOnly) => ResponseCodeKind::ReadOnly,
                            Some(ResponseCode::ReadWrite) => ResponseCodeKind::ReadWrite,
                            _ => ResponseCodeKind::Other,
                        });
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

    /// Like `serve`, for the write-back connection only. It allows SELECT and
    /// STORE, which `serve` forbids for every read-only sync test: the guarantee
    /// that header sync never writes stays enforced by all of those tests.
    async fn serve_writable(server: DuplexStream, script: Vec<(String, String)>) {
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
            assert!(!command.starts_with("EXPUNGE") && !command.starts_with("UID EXPUNGE"));
            assert!(!command.contains("BODY["));
            server
                .get_mut()
                .write_all(reply.replace("{tag}", tag).as_bytes())
                .await
                .unwrap();
        }
    }

    pub(super) async fn writable_session(
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
        let server = tokio::spawn(serve_writable(server, script));
        let session = test_session(Box::new(client), &login()).await.unwrap();
        (session, server)
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
            let mut session = test_session(Box::new(client), &login()).await?;
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
    async fn provider_policy_is_redacted_at_login_capability_id_and_directory_boundaries() {
        let login = ("LOGIN", "{tag} OK\r\n");
        let caps = ("CAPABILITY", "* CAPABILITY IMAP4rev1 ID\r\n{tag} OK\r\n");
        let policy =
            "{tag} NO Unsafe Login. Please contact kefu@188.com for help private-marker\r\n";
        for script in [
            vec![("LOGIN", policy)],
            vec![(
                "LOGIN",
                "* BYE Unsafe Login. Please contact kefu@188.com for help private-marker\r\n",
            )],
            vec![login, ("CAPABILITY", policy)],
            vec![login, caps, ("ID", policy)],
            vec![login, caps, ("ID", "{tag} OK\r\n"), ("LIST", policy)],
        ] {
            let error = run(script).await.unwrap_err();
            assert_eq!(error, ImapError::ProviderPolicy);
            assert!(!format!("{error:?} {error}").contains("private-marker"));
            assert!(!format!("{error:?} {error}").contains("kefu"));
        }
        let (mut session, server) = header_session(vec![("NOOP".into(), policy.into())]).await;
        assert_eq!(session.verify_noop().await, Err(ImapError::ProviderPolicy));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn unknown_rejections_and_positive_text_do_not_become_provider_policy() {
        let unknown = "{tag} NO Unsafe Login may mean something else private-marker\r\n";
        assert!(matches!(
            run(vec![("LOGIN", unknown)]).await,
            Err(ImapError::Authentication(_))
        ));
        let (mut session, server) = header_session(vec![("NOOP".into(), unknown.into())]).await;
        assert_eq!(session.verify_noop().await, Err(ImapError::Rejected));
        server.await.unwrap();
        let (mut session, server) = header_session(vec![(
            "NOOP".into(),
            "{tag} OK Unsafe Login. Please contact kefu@188.com for help\r\n".into(),
        )])
        .await;
        session.verify_noop().await.unwrap();
        server.await.unwrap();
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
                exists: Some(3)
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
    async fn lossy_status_names_use_counts_from_each_original_mailbox() {
        let (mut session, server) = header_session(vec![
            (
                "STATUS \"&XfJT0ZAB-\" ".into(),
                "* STATUS \"???\" (MESSAGES 999 UNSEEN 888 UIDNEXT 999 UIDVALIDITY 999)\r\n{tag} OK\r\n".into(),
            ),
            (
                "EXAMINE \"&XfJT0ZAB-\"".into(),
                "* 3 EXISTS\r\n* OK [UIDVALIDITY 9] valid\r\n* OK [UIDNEXT 43] next\r\n* OK [UNSEEN 42] first unread sequence\r\n{tag} OK [READ-ONLY]\r\n".into(),
            ),
            (
                "UID SEARCH UNSEEN".into(),
                "* SEARCH 7 19 7\r\n{tag} OK\r\n".into(),
            ),
            (
                "STATUS \"&g0l6P3ux-\" ".into(),
                "* STATUS \"???\" (MESSAGES 999 UNSEEN 888)\r\n{tag} OK\r\n".into(),
            ),
            (
                "EXAMINE \"&g0l6P3ux-\"".into(),
                "* 0 EXISTS\r\n* OK [UIDVALIDITY 10] valid\r\n{tag} OK [READ-ONLY]\r\n".into(),
            ),
            ("UID SEARCH UNSEEN".into(), "* SEARCH\r\n{tag} OK\r\n".into()),
        ])
        .await;
        let (sent, examined) = session.status_or_examine("&XfJT0ZAB-").await.unwrap();
        assert_eq!(
            sent,
            FolderStatus {
                messages: 3,
                unseen: 2,
                uid_next: Some(43),
                uid_validity: Some(9),
            }
        );
        assert_eq!(examined.unwrap().uid_validity, 9);
        let (drafts, examined) = session.status_or_examine("&g0l6P3ux-").await.unwrap();
        assert_eq!(
            drafts,
            FolderStatus {
                messages: 0,
                unseen: 0,
                uid_next: None,
                uid_validity: Some(10),
            }
        );
        assert_eq!(examined.unwrap().uid_validity, 10);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn completed_status_with_missing_data_can_use_examine() {
        for reply in [
            "{tag} OK\r\n",
            "* STATUS INBOX (MESSAGES 99)\r\n{tag} OK\r\n",
        ] {
            let (mut session, server) = header_session(vec![
                ("STATUS".into(), reply.into()),
                (
                    "EXAMINE".into(),
                    "* 1 EXISTS\r\n* OK [UIDVALIDITY 9] valid\r\n{tag} OK\r\n".into(),
                ),
                (
                    "UID SEARCH UNSEEN".into(),
                    "* SEARCH 7\r\n{tag} OK\r\n".into(),
                ),
            ])
            .await;
            let (status, _) = session.status_or_examine("INBOX").await.unwrap();
            assert_eq!((status.messages, status.unseen), (1, 1));
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn rejected_or_interrupted_status_never_attempts_the_fallback() {
        for reply in [
            "{tag} NO private\r\n",
            "{tag} BAD private\r\n",
            "* STATUS INBOX (MESSAGES 3 UNSEEN 2)\r\n{tag} NO private\r\n",
            "* STATUS INBOX (MESSAGES 3 UNSEEN 2)\r\n",
            "* BYE private\r\n",
        ] {
            let (mut session, server) = header_session(vec![("STATUS".into(), reply.into())]).await;
            assert!(session.status_or_examine("INBOX").await.is_err());
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn fallback_requires_complete_examine_and_unread_enumeration() {
        let valid_examine = "* 3 EXISTS\r\n* OK [UIDVALIDITY 9] valid\r\n{tag} OK\r\n";
        for (examine, search) in [
            ("* OK [UIDVALIDITY 9] valid\r\n{tag} OK\r\n", None),
            ("* 3 EXISTS\r\n{tag} OK\r\n", None),
            (
                "* 3 EXISTS\r\n* OK [UIDVALIDITY 9] valid\r\n{tag} NO private\r\n",
                None,
            ),
            ("* 3 EXISTS\r\n* OK [UIDVALIDITY 9] valid\r\n", None),
            (valid_examine, Some("{tag} OK\r\n")),
            (valid_examine, Some("* SEARCH 7\r\n{tag} NO private\r\n")),
            (valid_examine, Some("* SEARCH 7\r\n")),
            (valid_examine, Some("* SEARCH 0\r\n{tag} OK\r\n")),
        ] {
            let mut script = vec![
                ("STATUS".into(), "{tag} OK\r\n".into()),
                ("EXAMINE".into(), examine.into()),
            ];
            if let Some(search) = search {
                script.push(("UID SEARCH UNSEEN".into(), search.into()));
            }
            let (mut session, server) = header_session(script).await;
            assert!(session.status_or_examine("INBOX").await.is_err());
            server.await.unwrap();
        }
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

    // ---- write-back ----------------------------------------------------

    fn select_reply(permanent: &str) -> String {
        format!(
            "* 3 EXISTS\r\n* FLAGS (\\Deleted \\Seen)\r\n* OK [PERMANENTFLAGS ({permanent})] limited\r\n\
             * OK [UIDVALIDITY 9] valid\r\n{{tag}} OK [READ-WRITE] SELECT completed\r\n"
        )
    }

    #[tokio::test]
    async fn select_reads_which_flags_the_server_will_keep() {
        // 2925: only \\Deleted and \\Seen, no \\Flagged.
        let (mut session, server) = writable_session(vec![
            ("SELECT \"INBOX\"".into(), select_reply("\\Deleted \\Seen")),
            (
                "SELECT \"All\"".into(),
                select_reply("\\Deleted \\Seen \\Flagged \\*"),
            ),
            (
                "SELECT \"Quiet\"".into(),
                "* OK [UIDVALIDITY 4] x\r\n{tag} OK [READ-WRITE] done\r\n".into(),
            ),
            (
                "SELECT \"Ro\"".into(),
                "* OK [UIDVALIDITY 4] x\r\n{tag} OK [READ-ONLY] done\r\n".into(),
            ),
        ])
        .await;
        let inbox = session.select("INBOX").await.unwrap();
        assert_eq!(inbox.uid_validity, 9);
        assert_eq!(inbox.keeps("\\Seen"), Some(true));
        assert_eq!(inbox.keeps("\\Flagged"), Some(false));
        let all = session.select("All").await.unwrap();
        assert_eq!(all.keeps("\\flagged"), Some(true));
        // No PERMANENTFLAGS at all: unknown, not "no".
        let quiet = session.select("Quiet").await.unwrap();
        assert_eq!(quiet.keeps("\\Flagged"), None);
        assert!(!quiet.read_only);
        assert!(session.select("Ro").await.unwrap().read_only);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn select_without_a_uidvalidity_is_refused() {
        let (mut session, server) = writable_session(vec![(
            "SELECT".into(),
            "* 1 EXISTS\r\n{tag} OK [READ-WRITE] done\r\n".into(),
        )])
        .await;
        assert!(matches!(
            session.select("INBOX").await,
            Err(ImapError::Mailbox(_))
        ));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn store_sets_and_clears_one_flag_and_returns_what_the_server_now_has() {
        let (mut session, server) = writable_session(vec![
            (
                "UID STORE 42 +FLAGS (\\Seen)".into(),
                "* 5 FETCH (UID 42 FLAGS (\\Seen))\r\n{tag} OK stored\r\n".into(),
            ),
            (
                "UID STORE 42 -FLAGS (\\Seen)".into(),
                "* 5 FETCH (UID 42 FLAGS ())\r\n{tag} OK stored\r\n".into(),
            ),
            (
                "UID STORE 7 +FLAGS (\\Flagged)".into(),
                "* 2 FETCH (UID 7 FLAGS (\\Seen \\Flagged))\r\n{tag} OK stored\r\n".into(),
            ),
        ])
        .await;
        assert_eq!(
            session.store_flag(42, "\\Seen", true).await.unwrap(),
            ["\\Seen"]
        );
        assert!(
            session
                .store_flag(42, "\\Seen", false)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            session
                .store_flag(7, "\\Flagged", true)
                .await
                .unwrap()
                .len(),
            2
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn a_rejection_is_an_answer_and_an_unconfirmed_store_is_not_success() {
        let (mut session, server) = writable_session(vec![
            (
                "UID STORE 1".into(),
                "{tag} NO [READONLY] cannot write\r\n".into(),
            ),
            // OK but no FETCH data: the message may be gone, nothing is confirmed.
            ("UID STORE 2".into(), "{tag} OK done\r\n".into()),
            // Confirmed, but for a different UID than the one asked about.
            (
                "UID STORE 3".into(),
                "* 1 FETCH (UID 99 FLAGS (\\Seen))\r\n{tag} OK done\r\n".into(),
            ),
            // Acknowledged yet the flag is not there: the server ignored it.
            (
                "UID STORE 4".into(),
                "* 1 FETCH (UID 4 FLAGS ())\r\n{tag} OK done\r\n".into(),
            ),
        ])
        .await;
        let rejected = session.store_flag(1, "\\Seen", true).await.unwrap_err();
        assert_eq!(rejected, ImapError::Rejected);
        assert!(rejected.session_usable());
        for uid in [2, 3, 4] {
            let error = session.store_flag(uid, "\\Seen", true).await.unwrap_err();
            assert!(
                matches!(error, ImapError::Mailbox(_)),
                "uid {uid}: {error:?}"
            );
            assert!(error.session_usable());
        }
        server.await.unwrap();
    }

    #[tokio::test]
    async fn only_the_two_managed_flags_and_valid_uids_ever_reach_the_wire() {
        // No scripted commands: anything sent would fail the script check.
        let (mut session, server) = writable_session(vec![]).await;
        for bad in [
            "\\Deleted",
            "\\Seen \\Deleted",
            "\\Seen)\r\nA1 LOGOUT",
            "$Junk",
            "",
        ] {
            assert!(session.store_flag(1, bad, true).await.is_err(), "{bad:?}");
        }
        assert!(session.store_flag(0, "\\Seen", true).await.is_err());
        drop(session);
        let _ = server.await;
    }
}
