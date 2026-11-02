//! `mailx-mcp`: stdio bridge between an MCP client and a running MailX.
//!
//! MCP clients launch this program and speak MCP over its stdin/stdout. The
//! bridge reads the discovery file written by MailX, connects to the local
//! endpoint it names, proves it holds the token, and then copies bytes in both
//! directions. It never parses MCP while connected and never links
//! `mailx-core`; the only file it reads is the discovery file.
//!
//! When MailX cannot be reached the bridge answers the client's first request
//! with a JSON-RPC error carrying `CORE_UNAVAILABLE` (see `docs/contracts/mcp.md`)
//! and exits with status 69, so the client shows a readable message instead of
//! a bare "server closed".

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{self, AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};

mod connect;

/// Process exit status when MailX is unavailable (`EX_UNAVAILABLE`).
const EXIT_UNAVAILABLE: u8 = 69;
/// Exit status for bad command-line usage (`EX_USAGE`).
const EXIT_USAGE: u8 = 64;
/// JSON-RPC error code used for `CORE_UNAVAILABLE`. Inside the implementation
/// defined server-error range (-32000..-32099).
const CORE_UNAVAILABLE_CODE: i64 = -32001;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const DEGRADED_WAIT: Duration = Duration::from_secs(30);
/// How long replies may still drain after stdin reaches EOF.
const DRAIN_GRACE: Duration = Duration::from_secs(2);
const MAX_ACK_BYTES: usize = 256;
const HINT: &str = "MailX 未运行，或未启用本地 MCP。请先启动 MailX，并在设置中开启 MCP。 / \
MailX is not running or local MCP is turned off. Start MailX and enable MCP in Settings.";

/// File name MailX writes in its data directory.
const DISCOVERY_FILE_NAME: &str = "mcp-discovery.json";
/// Same override variable the desktop host honours for its data directory.
const DATA_DIR_OVERRIDE: &str = "MAILX_DATA_DIR";
const DISCOVERY_OVERRIDE: &str = "MAILX_MCP_DISCOVERY";
const USAGE: &str = "usage: mailx-mcp [--discovery <path>]\n\
  --discovery <path>   discovery file written by MailX (default: MAILX_MCP_DISCOVERY, \
else the MailX data directory)";

#[derive(Debug, PartialEq, Eq)]
enum Reason {
    DiscoveryMissing,
    DiscoveryInvalid,
    ConnectFailed,
    HandshakeRejected,
}

impl Reason {
    fn as_str(&self) -> &'static str {
        match self {
            Self::DiscoveryMissing => "discoveryMissing",
            Self::DiscoveryInvalid => "discoveryInvalid",
            Self::ConnectFailed => "connectFailed",
            Self::HandshakeRejected => "handshakeRejected",
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Discovery {
    endpoint: String,
    token: String,
}

fn main() -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("mailx-mcp: cannot start the runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    let code = runtime.block_on(run_bridge());
    // `tokio::io::stdin()` reads on a blocking thread that cannot be
    // cancelled; a normal runtime drop would wait for it forever when the
    // server closed first while the client keeps stdin open.
    runtime.shutdown_background();
    code
}

async fn run_bridge() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let flag = match parse_args(&args) {
        Ok(flag) => flag,
        Err(message) => {
            eprintln!("mailx-mcp: {message}\n{USAGE}");
            return ExitCode::from(EXIT_USAGE);
        }
    };
    let path = discovery_path(flag, &|name| std::env::var_os(name));
    let Some(path) = path else {
        return unavailable(
            Reason::DiscoveryMissing,
            "cannot work out where MailX keeps its data",
        )
        .await;
    };
    match run(&path).await {
        Ok(()) => ExitCode::SUCCESS,
        Err((reason, detail)) => unavailable(reason, &detail).await,
    }
}

async fn run(path: &std::path::Path) -> Result<(), (Reason, String)> {
    let discovery = read_discovery(path)?;
    let mut stream = tokio::time::timeout(HANDSHAKE_TIMEOUT, connect::open(&discovery.endpoint))
        .await
        .map_err(|_| (Reason::ConnectFailed, "connecting timed out".to_owned()))?
        .map_err(|error| (Reason::ConnectFailed, error.to_string()))?;
    tokio::time::timeout(HANDSHAKE_TIMEOUT, handshake(&mut stream, &discovery.token))
        .await
        .map_err(|_| (Reason::HandshakeRejected, "handshake timed out".to_owned()))??;
    relay(&mut stream).await;
    Ok(())
}

/// Copies stdin to the endpoint and the endpoint to stdout. MCP stdio ends
/// when the client closes stdin, and a Windows named pipe has no half-close,
/// so on stdin EOF the bridge flushes, gives the server `DRAIN_GRACE` to finish
/// its last replies, and then drops the connection so the server frees the
/// session. If the server closes first, the relay ends at once.
async fn relay<S>(stream: &mut S)
where
    S: io::AsyncRead + io::AsyncWrite + Unpin,
{
    let (mut from_server, mut to_server) = io::split(stream);
    let upstream = async {
        let _ = io::copy(&mut io::stdin(), &mut to_server).await;
        let _ = to_server.flush().await;
    };
    let mut downstream = Box::pin(async {
        let mut stdout = io::stdout();
        let _ = io::copy(&mut from_server, &mut stdout).await;
        let _ = stdout.flush().await;
    });
    tokio::select! {
        () = &mut downstream => {}
        () = upstream => {
            let _ = tokio::time::timeout(DRAIN_GRACE, &mut downstream).await;
        }
    }
}

fn read_discovery(path: &std::path::Path) -> Result<Discovery, (Reason, String)> {
    let bytes = std::fs::read(path).map_err(|error| {
        let reason = if error.kind() == std::io::ErrorKind::NotFound {
            Reason::DiscoveryMissing
        } else {
            Reason::DiscoveryInvalid
        };
        (reason, format!("{}: {error}", path.display()))
    })?;
    parse_discovery(&bytes).ok_or_else(|| {
        (
            Reason::DiscoveryInvalid,
            format!("{}: unreadable", path.display()),
        )
    })
}

fn parse_discovery(bytes: &[u8]) -> Option<Discovery> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    if value.get("version")?.as_u64()? != 1 {
        return None;
    }
    let endpoint = value.get("endpoint")?.as_str()?.to_owned();
    let token = value.get("token")?.as_str()?.to_owned();
    (!endpoint.is_empty() && !token.is_empty()).then_some(Discovery { endpoint, token })
}

/// Sends the hello line and reads the answer one byte at a time, so no MCP
/// bytes that follow the answer are consumed.
async fn handshake<S>(stream: &mut S, token: &str) -> Result<(), (Reason, String)>
where
    S: io::AsyncRead + io::AsyncWrite + Unpin,
{
    let rejected = |detail: &str| (Reason::HandshakeRejected, detail.to_owned());
    let hello = format!("{}\n", json!({"bridge": 1, "token": token}));
    stream
        .write_all(hello.as_bytes())
        .await
        .map_err(|_| rejected("could not send the handshake"))?;
    let mut ack = Vec::new();
    loop {
        match stream.read_u8().await {
            Ok(b'\n') => break,
            Ok(byte) if ack.len() < MAX_ACK_BYTES => ack.push(byte),
            _ => return Err(rejected("connection closed during the handshake")),
        }
    }
    if ack == b"{\"ok\":true}" {
        Ok(())
    } else {
        Err(rejected("MailX refused the token or has no free slot"))
    }
}

fn parse_args(args: &[OsString]) -> Result<Option<PathBuf>, String> {
    let mut path = None;
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.to_str() {
            Some("--discovery") => {
                let value = iter.next().ok_or("--discovery needs a path")?;
                path = Some(PathBuf::from(value));
            }
            Some(text) if text.starts_with("--discovery=") => {
                path = Some(PathBuf::from(&text["--discovery=".len()..]));
            }
            _ => return Err(format!("unexpected argument {arg:?}")),
        }
    }
    Ok(path)
}

/// Resolution order: `--discovery`, `MAILX_MCP_DISCOVERY`, then the MailX
/// data directory.
///
/// COUPLING: the data directory mirrors `desktop/src/storage_location.rs`
/// (`MAILX_DATA_DIR` override, otherwise `<platform data base>/MailX`). If the
/// desktop host changes where it keeps data, change `default_data_dir` too.
fn discovery_path(
    flag: Option<PathBuf>,
    env: &dyn Fn(&str) -> Option<OsString>,
) -> Option<PathBuf> {
    let non_empty = |name: &str| env(name).filter(|value| !value.is_empty());
    if let Some(path) = flag {
        return Some(path);
    }
    if let Some(path) = non_empty(DISCOVERY_OVERRIDE) {
        return Some(PathBuf::from(path));
    }
    let data_dir = match non_empty(DATA_DIR_OVERRIDE) {
        Some(dir) => PathBuf::from(dir),
        None => default_data_dir(env)?,
    };
    Some(data_dir.join(DISCOVERY_FILE_NAME))
}

/// `<base>/MailX`, where base is Application Support (macOS), `%APPDATA%`
/// (Windows) or `$XDG_DATA_HOME` / `~/.local/share` (Linux and other Unix).
fn default_data_dir(env: &dyn Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let non_empty = |name: &str| {
        env(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    let base = if cfg!(windows) {
        non_empty("APPDATA")?
    } else if cfg!(target_os = "macos") {
        non_empty("HOME")?.join("Library/Application Support")
    } else {
        non_empty("XDG_DATA_HOME")
            .filter(|dir| dir.is_absolute())
            .or_else(|| non_empty("HOME").map(|home| home.join(".local/share")))?
    };
    Some(base.join("MailX"))
}

/// MailX cannot be reached: tell the client why, once, then exit 69.
async fn unavailable(reason: Reason, detail: &str) -> ExitCode {
    eprintln!(
        "mailx-mcp: CORE_UNAVAILABLE ({}): {detail}. {HINT}",
        reason.as_str()
    );
    let _ = tokio::time::timeout(DEGRADED_WAIT, answer_first_request(reason)).await;
    ExitCode::from(EXIT_UNAVAILABLE)
}

/// Reads stdin lines until one is a JSON-RPC request (has an `id`), answers it
/// with the error, and returns. Notifications and junk get no reply. This is
/// the only place the bridge looks inside MCP, and only for the `id`.
async fn answer_first_request(reason: Reason) {
    let mut lines = BufReader::new(io::stdin()).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(Value::Object(message)) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(id) = message.get("id").filter(|id| !id.is_null()) else {
            continue;
        };
        let reply = unavailable_reply(id, &reason);
        let mut out = io::stdout();
        let _ = out.write_all(format!("{reply}\n").as_bytes()).await;
        let _ = out.flush().await;
        return;
    }
}

fn unavailable_reply(id: &Value, reason: &Reason) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": CORE_UNAVAILABLE_CODE,
            "message": format!("CORE_UNAVAILABLE: {HINT}"),
            "data": {"code": "CORE_UNAVAILABLE", "reason": reason.as_str()},
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<OsString> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name| {
            pairs
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| OsString::from(value))
        }
    }

    #[test]
    fn flag_beats_env_beats_data_dir() {
        let env = env_of(&[
            (DISCOVERY_OVERRIDE, "/from/env.json"),
            (DATA_DIR_OVERRIDE, "/data"),
        ]);
        assert_eq!(
            discovery_path(Some("/flag.json".into()), &env),
            Some(PathBuf::from("/flag.json"))
        );
        assert_eq!(
            discovery_path(None, &env),
            Some(PathBuf::from("/from/env.json"))
        );
        let env = env_of(&[(DATA_DIR_OVERRIDE, "/data")]);
        assert_eq!(
            discovery_path(None, &env),
            Some(PathBuf::from("/data").join(DISCOVERY_FILE_NAME))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_default_matches_the_desktop_host() {
        let env = env_of(&[("HOME", "/Users/me")]);
        assert_eq!(
            discovery_path(None, &env),
            Some(PathBuf::from(
                "/Users/me/Library/Application Support/MailX/mcp-discovery.json"
            ))
        );
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn linux_default_matches_the_desktop_host() {
        let env = env_of(&[("HOME", "/home/me")]);
        assert_eq!(
            discovery_path(None, &env),
            Some(PathBuf::from(
                "/home/me/.local/share/MailX/mcp-discovery.json"
            ))
        );
        let env = env_of(&[("HOME", "/home/me"), ("XDG_DATA_HOME", "/xdg")]);
        assert_eq!(
            discovery_path(None, &env),
            Some(PathBuf::from("/xdg/MailX/mcp-discovery.json"))
        );
    }

    #[test]
    fn no_home_means_no_default() {
        assert_eq!(discovery_path(None, &env_of(&[])), None);
    }

    #[test]
    fn args_accept_both_flag_forms_and_reject_others() {
        let args = |list: &[&str]| list.iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(parse_args(&args(&[])), Ok(None));
        assert_eq!(
            parse_args(&args(&["--discovery", "/a"])),
            Ok(Some(PathBuf::from("/a")))
        );
        assert_eq!(
            parse_args(&args(&["--discovery=/b"])),
            Ok(Some(PathBuf::from("/b")))
        );
        assert!(parse_args(&args(&["--discovery"])).is_err());
        assert!(parse_args(&args(&["--nope"])).is_err());
    }

    #[test]
    fn discovery_parsing_requires_version_endpoint_and_token() {
        assert_eq!(
            parse_discovery(
                br#"{"version":1,"endpoint":"/s","token":"t","pid":1,"instanceId":"i"}"#
            ),
            Some(Discovery {
                endpoint: "/s".into(),
                token: "t".into()
            })
        );
        for bad in [
            &br#"{"version":2,"endpoint":"/s","token":"t"}"#[..],
            br#"{"version":1,"endpoint":"","token":"t"}"#,
            br#"{"version":1,"endpoint":"/s"}"#,
            b"nope",
        ] {
            assert_eq!(parse_discovery(bad), None);
        }
    }

    #[test]
    fn the_error_reply_echoes_the_id_and_carries_the_code() {
        let reply = unavailable_reply(&json!(7), &Reason::ConnectFailed);
        assert_eq!(reply["id"], 7);
        assert_eq!(reply["error"]["code"], CORE_UNAVAILABLE_CODE);
        assert!(
            reply["error"]["message"]
                .as_str()
                .unwrap()
                .starts_with("CORE_UNAVAILABLE")
        );
        assert_eq!(reply["error"]["data"]["code"], "CORE_UNAVAILABLE");
        assert_eq!(reply["error"]["data"]["reason"], "connectFailed");
    }
}
