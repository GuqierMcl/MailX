//! IMAP client operations used by sync. Only what folder sync needs so far:
//! connect along the account route, log in, list folders and read their status.

pub(crate) mod utf7;

use std::fmt;
use std::future::Future;
use std::time::Duration;

use async_imap::Client;
use async_imap::error::Error as ImapProtocolError;
use async_imap::types::NameAttribute;
use futures_util::TryStreamExt;
use tokio::time::timeout;

use crate::accounts::types::TlsMode;
use crate::net::{self, BoxedStream, NetError, NetRoute};
use crate::secrets::SecretValue;

/// Upper bound for one command round trip; a server that stalls longer is treated as gone.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(60);

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
    /// `None` for folders that cannot be selected and so have no status.
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
    Timeout,
}

impl fmt::Display for ImapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Net(error) => error.fmt(formatter),
            Self::Authentication(reason) => write!(formatter, "login rejected: {reason}"),
            Self::Unsupported(reason) => write!(formatter, "unsupported by server: {reason}"),
            Self::Protocol(reason) => write!(formatter, "protocol error: {reason}"),
            Self::Timeout => formatter.write_str("server stopped responding"),
        }
    }
}

impl From<NetError> for ImapError {
    fn from(error: NetError) -> Self {
        Self::Net(error)
    }
}

impl From<ImapProtocolError> for ImapError {
    fn from(error: ImapProtocolError) -> Self {
        Self::Protocol(error.to_string())
    }
}

async fn step<T, E: Into<ImapError>>(
    future: impl Future<Output = Result<T, E>>,
) -> Result<T, ImapError> {
    timeout(COMMAND_TIMEOUT, future)
        .await
        .map_err(|_| ImapError::Timeout)?
        .map_err(Into::into)
}

/// Connects along `route`, logs in and returns every folder with its status.
/// Plaintext is only ever used to negotiate STARTTLS; credentials are sent after TLS.
pub(crate) async fn fetch_folders(
    route: &NetRoute,
    login: &ImapLogin,
) -> Result<Vec<RemoteFolder>, ImapError> {
    let stream = net::connect(route, &login.host, login.port).await?;
    let client = match login.tls {
        TlsMode::Implicit => greet(net::upgrade_tls(stream, &login.host).await?).await?,
        TlsMode::StartTls => {
            let mut client = greet(stream).await?;
            step(client.run_command_and_check_ok("STARTTLS", None))
                .await
                .map_err(|_| ImapError::Unsupported("STARTTLS".into()))?;
            // No greeting follows STARTTLS; the session continues inside TLS.
            Client::new(net::upgrade_tls(client.into_inner(), &login.host).await?)
        }
    };
    list_folders(client, login).await
}

/// Reads the server greeting that opens every IMAP connection.
async fn greet(stream: BoxedStream) -> Result<Client<BoxedStream>, ImapError> {
    let mut client = Client::new(stream);
    let greeting = step(async {
        client
            .read_response()
            .await
            .map_err(|error| ImapError::Protocol(error.to_string()))
    })
    .await?;
    match greeting {
        Some(_) => Ok(client),
        None => Err(ImapError::Protocol(
            "connection closed before the greeting".into(),
        )),
    }
}

async fn list_folders(
    client: Client<BoxedStream>,
    login: &ImapLogin,
) -> Result<Vec<RemoteFolder>, ImapError> {
    let mut session = timeout(
        COMMAND_TIMEOUT,
        client.login(&login.username, login.password.expose_secret()),
    )
    .await
    .map_err(|_| ImapError::Timeout)?
    .map_err(|(error, _)| match error {
        ImapProtocolError::No(reason) | ImapProtocolError::Bad(reason) => {
            ImapError::Authentication(reason)
        }
        other => ImapError::from(other),
    })?;

    let capabilities = step(session.capabilities()).await?;
    if capabilities.has_str("ID") {
        // Some providers (NetEase 163/126 among them) refuse to open folders
        // for clients that do not identify themselves. Failure here is harmless.
        let _ = step(session.id([
            ("name", Some("MailX")),
            ("version", Some(env!("CARGO_PKG_VERSION"))),
        ]))
        .await;
    }

    let names: Vec<_> =
        step(async { session.list(Some(""), Some("*")).await?.try_collect().await }).await?;
    let mut folders = Vec::with_capacity(names.len());
    for name in &names {
        let attributes: Vec<String> = name.attributes().iter().map(attribute_name).collect();
        let selectable = !attributes.iter().any(|attribute| {
            attribute.eq_ignore_ascii_case("\\Noselect")
                || attribute.eq_ignore_ascii_case("\\NonExistent")
        });
        let status = if selectable {
            let mailbox =
                step(session.status(name.name(), "(MESSAGES UNSEEN UIDNEXT UIDVALIDITY)")).await?;
            Some(FolderStatus {
                messages: mailbox.exists,
                unseen: mailbox.unseen.unwrap_or(0),
                uid_next: mailbox.uid_next,
                uid_validity: mailbox.uid_validity,
            })
        } else {
            None
        };
        folders.push(RemoteFolder {
            name: name.name().to_owned(),
            delimiter: name.delimiter().map(str::to_owned),
            attributes,
            status,
        });
    }
    let _ = step(session.logout()).await;
    Ok(folders)
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
    async fn serve(server: DuplexStream, script: Vec<(&'static str, &'static str)>) {
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
                command.starts_with(expected),
                "expected {expected:?}, got {command:?}"
            );
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
        let server = tokio::spawn(serve(server, script));
        let result = list_folders(greet(Box::new(client)).await?, &login()).await;
        server.await.unwrap();
        result
    }

    #[tokio::test]
    async fn lists_folders_with_attributes_and_status() {
        let folders = run(vec![
            (r#"LOGIN "lin@example.com" "app \"pass\"""#, "{tag} OK LOGIN completed\r\n"),
            ("CAPABILITY", "* CAPABILITY IMAP4rev1 ID SPECIAL-USE\r\n{tag} OK done\r\n"),
            ("ID (\"name\" \"MailX\"", "* ID NIL\r\n{tag} OK ID completed\r\n"),
            (
                "LIST \"\" *",
                "* LIST (\\HasNoChildren) \"/\" INBOX\r\n\
                 * LIST (\\HasNoChildren \\Sent) \"/\" \"&XfJT0ZAB-\"\r\n\
                 * LIST (\\Noselect \\HasChildren) \"/\" \"Clients\"\r\n\
                 * LIST (\\HasNoChildren) \"/\" \"Clients/East\"\r\n\
                 {tag} OK LIST completed\r\n",
            ),
            ("STATUS \"INBOX\"", "* STATUS INBOX (MESSAGES 12 UNSEEN 3 UIDNEXT 40 UIDVALIDITY 7)\r\n{tag} OK\r\n"),
            ("STATUS \"&XfJT0ZAB-\"", "* STATUS \"&XfJT0ZAB-\" (MESSAGES 4 UNSEEN 0 UIDNEXT 5 UIDVALIDITY 7)\r\n{tag} OK\r\n"),
            ("STATUS \"Clients/East\"", "* STATUS \"Clients/East\" (MESSAGES 1 UNSEEN 1 UIDNEXT 2 UIDVALIDITY 9)\r\n{tag} OK\r\n"),
            ("LOGOUT", "* BYE\r\n{tag} OK LOGOUT completed\r\n"),
        ])
        .await
        .unwrap();

        let names: Vec<_> = folders.iter().map(|folder| folder.name.as_str()).collect();
        assert_eq!(names, ["INBOX", "&XfJT0ZAB-", "Clients", "Clients/East"]);
        assert_eq!(
            folders[0].status,
            Some(FolderStatus {
                messages: 12,
                unseen: 3,
                uid_next: Some(40),
                uid_validity: Some(7)
            })
        );
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
            matches!(error, ImapError::Authentication(reason) if reason.contains("Invalid credentials"))
        );
    }

    #[tokio::test]
    async fn skips_id_when_the_server_does_not_offer_it() {
        let folders = run(vec![
            ("LOGIN", "{tag} OK\r\n"),
            ("CAPABILITY", "* CAPABILITY IMAP4rev1\r\n{tag} OK\r\n"),
            ("LIST", "* LIST () \".\" INBOX\r\n{tag} OK\r\n"),
            (
                "STATUS",
                "* STATUS INBOX (MESSAGES 0 UNSEEN 0)\r\n{tag} OK\r\n",
            ),
            ("LOGOUT", "{tag} OK\r\n"),
        ])
        .await
        .unwrap();
        assert_eq!(folders.len(), 1);
    }
}
