//! Host-owned integration with local agent configurations. The webview selects
//! a known agent; it can never choose a file path, command, or import URL.
mod bridge_check;
mod config_edit;

use crate::{
    commands::{EmptyArgs, authorize, parse, respond},
    mcp_sidecar::McpClientConfig,
    state::DesktopState,
};
use base64::Engine;
use mailx_core::{ApiResult, AppError, ErrorCode};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::Mutex};
use tauri::{State, WebviewWindow};
use tauri_plugin_opener::OpenerExt;

static CONFIG_WRITE: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum McpAgentId {
    ClaudeCode,
    Codex,
    Cursor,
    OpenCode,
    Trae,
    DeepseekHarness,
}
const AGENTS: [McpAgentId; 6] = [
    McpAgentId::ClaudeCode,
    McpAgentId::Codex,
    McpAgentId::Cursor,
    McpAgentId::OpenCode,
    McpAgentId::Trae,
    McpAgentId::DeepseekHarness,
];

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct McpAgent {
    id: McpAgentId,
    name: &'static str,
    method: &'static str,
    configuration: &'static str,
    available: bool,
    snippet: String,
    docs_url: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    config_path: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct McpSetupResult {
    state: &'static str,
    agent: McpAgent,
}

#[derive(Clone)]
struct Locations {
    home: PathBuf,
    codex: PathBuf,
    opencode: PathBuf,
}
impl Locations {
    fn current() -> Result<Self, ()> {
        let home = std::env::home_dir().ok_or(())?;
        let codex = std::env::var_os("CODEX_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codex"));
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        if !codex.is_absolute() || !config.is_absolute() {
            return Err(());
        }
        Ok(Self {
            home,
            codex,
            opencode: config.join("opencode"),
        })
    }
    fn path(&self, id: McpAgentId) -> Result<Option<PathBuf>, ()> {
        Ok(match id {
            McpAgentId::ClaudeCode => {
                // The alternate location is not stable across Claude versions.
                // Do not write a guessed path when the client has an override.
                if std::env::var_os("CLAUDE_CONFIG_DIR").is_some_and(|v| !v.is_empty()) {
                    return Err(());
                }
                Some(self.home.join(".claude.json"))
            }
            McpAgentId::Codex => Some(self.codex.join("config.toml")),
            McpAgentId::Cursor => Some(self.home.join(".cursor").join("mcp.json")),
            McpAgentId::OpenCode => {
                let json = self.opencode.join("opencode.json");
                let jsonc = self.opencode.join("opencode.jsonc");
                if json.exists() && jsonc.exists() {
                    return Err(());
                }
                Some(if json.exists() { json } else { jsonc })
            }
            _ => None,
        })
    }
}

fn entry(config: &McpClientConfig, id: McpAgentId) -> Value {
    if id == McpAgentId::OpenCode {
        let command: Vec<&str> = std::iter::once(config.command.as_str())
            .chain(config.args.iter().map(String::as_str))
            .collect();
        json!({"type":"local","command":command})
    } else {
        json!({"command":config.command,"args":config.args})
    }
}

fn json_path(id: McpAgentId, text: &str) -> Result<Vec<&'static str>, ()> {
    if id != McpAgentId::OpenCode {
        return Ok(vec!["mcpServers", "mailx"]);
    }
    let mcp = config_edit::json_entry(text, &["mcp"])?;
    // V1 stores server names directly under mcp. Keep an existing V1 layout.
    let legacy = mcp.as_ref().and_then(Value::as_object).is_some_and(|mcp| {
        !mcp.contains_key("servers")
            && mcp.values().any(|value| {
                matches!(
                    value.get("type").and_then(Value::as_str),
                    Some("local" | "remote")
                )
            })
    });
    Ok(if legacy {
        vec!["mcp", "mailx"]
    } else {
        vec!["mcp", "servers", "mailx"]
    })
}

fn current_entry(id: McpAgentId, text: &str) -> Result<Option<Value>, ()> {
    if id == McpAgentId::Codex {
        config_edit::toml_entry(text)
    } else {
        config_edit::json_entry(text, &json_path(id, text)?)
    }
}

fn same_connection(current: &Value, expected: &Value) -> bool {
    expected.as_object().is_some_and(|expected| {
        expected
            .iter()
            .all(|(key, value)| current.get(key) == Some(value))
    })
}

fn valid_transport(current: &Value, expected: &Value) -> bool {
    !config_edit::REMOTE_FIELDS
        .iter()
        .any(|key| current.get(key).is_some())
        && current.get("type").is_none_or(|value| {
            value
                == expected
                    .get("type")
                    .unwrap_or(&Value::String("stdio".into()))
        })
        && current
            .get("transport")
            .is_none_or(|value| value.as_str() == Some("stdio"))
}

fn matches_entry(current: &Value, expected: &Value) -> bool {
    same_connection(current, expected)
        && valid_transport(current, expected)
        && current.get("enabled") != Some(&Value::Bool(false))
        && current.get("disabled") != Some(&Value::Bool(true))
}

fn snippet(id: McpAgentId, config: &McpClientConfig, text: Option<&str>) -> String {
    let value = entry(config, id);
    if id == McpAgentId::Codex {
        return config_edit::edit_toml("", Some(&value)).unwrap_or_default();
    }
    if id == McpAgentId::DeepseekHarness {
        // A dedicated overlay, documented by the official MCP client example.
        // This is not the mcpServers format used by Claude/Cursor.
        return format!(
            "- insert:\n    - id: mailx-mcp\n      name: \"@deepseek-ai/dsh-mcp-client\"\n      config:\n        serverName: mailx\n        transport: stdio\n        command: {}\n        args: {}\n",
            serde_json::to_string(&config.command).unwrap_or_default(),
            serde_json::to_string(&config.args).unwrap_or_default()
        );
    }
    let wrapped = if id == McpAgentId::OpenCode {
        if text
            .and_then(|t| json_path(id, t).ok())
            .is_some_and(|p| p.len() == 2)
        {
            json!({"mcp":{"mailx":value}})
        } else {
            json!({"mcp":{"servers":{"mailx":value}}})
        }
    } else {
        json!({"mcpServers":{"mailx":value}})
    };
    serde_json::to_string_pretty(&wrapped).unwrap_or_default()
}

fn describe(id: McpAgentId, config: &McpClientConfig, locations: Option<&Locations>) -> McpAgent {
    let (name, method, docs_url) = match id {
        McpAgentId::ClaudeCode => (
            "Claude Code",
            "config",
            "https://code.claude.com/docs/en/mcp",
        ),
        McpAgentId::Codex => (
            "Codex",
            "config",
            "https://developers.openai.com/codex/mcp/",
        ),
        McpAgentId::Cursor => (
            "Cursor",
            "deeplink",
            "https://cursor.com/docs/mcp/install-links",
        ),
        McpAgentId::OpenCode => (
            "OpenCode",
            "config",
            "https://opencode.ai/docs/mcp-servers/",
        ),
        McpAgentId::Trae => (
            "Trae",
            "deeplink",
            "https://docs.trae.ai/ide/mcp-server-install-links",
        ),
        McpAgentId::DeepseekHarness => (
            "DeepSeek Harness",
            "manual",
            "https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/mcp/mcp-manager/README.md",
        ),
    };
    let path = locations.ok_or(()).and_then(|l| l.path(id));
    let contents = path
        .as_ref()
        .map_err(|_| ())
        .and_then(|p| p.as_ref().map_or(Ok(None), |p| config_edit::read(p)));
    let configuration = if method == "manual" {
        "manual"
    } else if method == "deeplink" && id == McpAgentId::Trae {
        "missing"
    } else {
        match &contents {
            Err(()) => "unavailable",
            Ok(None) => "missing",
            Ok(Some(text)) => match current_entry(id, text) {
                Ok(None) => "missing",
                Ok(Some(current)) if matches_entry(&current, &entry(config, id)) => "configured",
                Ok(Some(_)) => "conflict",
                Err(()) => "unavailable",
            },
        }
    };
    let available = method == "deeplink" || (method != "manual" && configuration != "unavailable");
    let config_path = path.ok().flatten().map(|p| {
        if let Some(locations) = locations {
            p.strip_prefix(&locations.home)
                .map(|tail| format!("~/{}", tail.display()))
                .unwrap_or_else(|_| p.to_string_lossy().into_owned())
        } else {
            p.to_string_lossy().into_owned()
        }
    });
    McpAgent {
        id,
        name,
        method,
        configuration,
        available,
        snippet: snippet(id, config, contents.ok().flatten().as_deref()),
        docs_url,
        config_path,
    }
}

fn import_url(id: McpAgentId, config: &McpClientConfig) -> Option<String> {
    let encoded = base64::engine::general_purpose::STANDARD
        .encode(serde_json::to_vec(&entry(config, id)).ok()?);
    let encoded =
        percent_encoding::utf8_percent_encode(&encoded, percent_encoding::NON_ALPHANUMERIC);
    match id {
        McpAgentId::Cursor => Some(format!(
            "cursor://anysphere.cursor-deeplink/mcp/install?name=mailx&config={encoded}"
        )),
        McpAgentId::Trae => Some(format!(
            "trae://trae.ai-ide/mcp-import?type=stdio&name=mailx&config={encoded}"
        )),
        _ => None,
    }
}

fn modify(
    id: McpAgentId,
    config: &McpClientConfig,
    locations: &Locations,
    replace: bool,
    remove: bool,
) -> Result<&'static str, ()> {
    let _guard = CONFIG_WRITE.lock().map_err(|_| ())?;
    let Some(path) = locations.path(id)? else {
        return Ok("unavailable");
    };
    let before = config_edit::read(&path)?;
    let empty = if id == McpAgentId::Codex { "" } else { "{}\n" };
    let text = before.as_deref().unwrap_or(empty);
    let current = current_entry(id, text)?;
    let mut expected = entry(config, id);
    if remove {
        let Some(ref current) = current else {
            return Ok("unchanged");
        };
        if !same_connection(current, &expected) || !valid_transport(current, &expected) {
            return Ok("conflict");
        }
    } else {
        if current
            .as_ref()
            .is_some_and(|current| matches_entry(current, &expected))
        {
            return Ok("unchanged");
        }
        if current.is_some() && !replace {
            return Ok("conflict");
        }
    }
    if !remove {
        // Explicit repair also re-enables this entry, preserving unrelated options.
        if current
            .as_ref()
            .is_some_and(|value| value.get("enabled").is_some())
        {
            expected["enabled"] = Value::Bool(true);
        }
        if current
            .as_ref()
            .is_some_and(|value| value.get("disabled").is_some())
        {
            expected["disabled"] = Value::Bool(false);
        }
    }
    let new_entry = (!remove).then_some(&expected);
    let after = if id == McpAgentId::Codex {
        config_edit::edit_toml(text, new_entry)?
    } else {
        config_edit::edit_json(text, &json_path(id, text)?, new_entry)?
    };
    config_edit::write(&path, before.as_deref(), &after)?;
    Ok(if remove { "removed" } else { "configured" })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SetupArgs {
    id: McpAgentId,
    #[serde(default)]
    replace: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentArgs {
    id: McpAgentId,
}

#[tauri::command]
pub(crate) async fn mcp_agents(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<Vec<McpAgent>>, ()> {
    let args = authorize(&window, "mcp_agents").and_then(|_| parse::<EmptyArgs>(request.body()));
    let result = match args {
        Err(error) => Err(error),
        Ok(_) => {
            let config = state
                .data_dir()
                .and_then(|dir| crate::mcp_sidecar::current_client_config(&dir));
            match config {
                Err(error) => Err(error),
                Ok(config) => tauri::async_runtime::spawn_blocking(move || {
                    let locations = Locations::current().ok();
                    AGENTS
                        .into_iter()
                        .map(|id| describe(id, &config, locations.as_ref()))
                        .collect()
                })
                .await
                .map_err(|_| AppError::new(ErrorCode::StorageUnavailable)),
            }
        }
    };
    Ok(respond(result))
}

#[tauri::command]
pub(crate) async fn mcp_agent_setup(
    app: tauri::AppHandle,
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<McpSetupResult>, ()> {
    let args =
        authorize(&window, "mcp_agent_setup").and_then(|_| parse::<SetupArgs>(request.body()));
    let result = match args {
        Err(error) => Err(error),
        Ok(args) => setup(&app, &state, args).await,
    };
    Ok(respond(result))
}
async fn setup(
    app: &tauri::AppHandle,
    state: &DesktopState,
    args: SetupArgs,
) -> Result<McpSetupResult, AppError> {
    let config = crate::mcp_sidecar::current_client_config(&state.data_dir()?)?;
    if let Some(url) = import_url(args.id, &config) {
        let state = if app.opener().open_url(url, None::<&str>).is_ok() {
            "handoff"
        } else {
            "unavailable"
        };
        let locations = Locations::current().ok();
        return Ok(McpSetupResult {
            state,
            agent: describe(args.id, &config, locations.as_ref()),
        });
    }
    tauri::async_runtime::spawn_blocking(move || {
        let locations = Locations::current().ok();
        let state = locations
            .as_ref()
            .and_then(|locations| modify(args.id, &config, locations, args.replace, false).ok())
            .unwrap_or("unavailable");
        McpSetupResult {
            state,
            agent: describe(args.id, &config, locations.as_ref()),
        }
    })
    .await
    .map_err(|_| AppError::new(ErrorCode::StorageUnavailable))
}

#[tauri::command]
pub(crate) async fn mcp_agent_remove(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<McpSetupResult>, ()> {
    let args =
        authorize(&window, "mcp_agent_remove").and_then(|_| parse::<AgentArgs>(request.body()));
    let result = match args {
        Err(error) => Err(error),
        Ok(args) => remove(&state, args.id).await,
    };
    Ok(respond(result))
}
async fn remove(state: &DesktopState, id: McpAgentId) -> Result<McpSetupResult, AppError> {
    let config = crate::mcp_sidecar::current_client_config(&state.data_dir()?)?;
    tauri::async_runtime::spawn_blocking(move || {
        let locations = Locations::current().ok();
        let state = locations
            .as_ref()
            .and_then(|locations| modify(id, &config, locations, false, true).ok())
            .unwrap_or("unavailable");
        McpSetupResult {
            state,
            agent: describe(id, &config, locations.as_ref()),
        }
    })
    .await
    .map_err(|_| AppError::new(ErrorCode::StorageUnavailable))
}

#[tauri::command]
pub(crate) async fn mcp_diagnose(
    window: WebviewWindow,
    state: State<'_, DesktopState>,
    request: tauri::ipc::Request<'_>,
) -> Result<ApiResult<bridge_check::McpDiagnostic>, ()> {
    let args = authorize(&window, "mcp_diagnose").and_then(|_| parse::<EmptyArgs>(request.body()));
    let result = match args {
        Err(error) => Err(error),
        Ok(_) => {
            if !state.mcp_enabled() {
                Ok(bridge_check::McpDiagnostic::failure("disabled"))
            } else {
                match state
                    .data_dir()
                    .and_then(|data_dir| crate::mcp_sidecar::current_client_config(&data_dir))
                {
                    Ok(config) => Ok(bridge_check::run(&config).await),
                    Err(_) => Ok(bridge_check::McpDiagnostic::failure("bridgeUnavailable")),
                }
            }
        }
    };
    Ok(respond(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    fn config() -> McpClientConfig {
        McpClientConfig {
            command: "/Applications/邮件 Work/mailx-mcp".into(),
            args: vec!["--discovery".into(), "/data/用户/mcp.json".into()],
            display: crate::mcp_sidecar::McpClientDisplay {
                command: "bridge".into(),
                discovery: "data".into(),
            },
        }
    }
    fn locations(home: &Path) -> Locations {
        Locations {
            home: home.into(),
            codex: home.join(".codex"),
            opencode: home.join(".config/opencode"),
        }
    }
    #[test]
    fn preserves_existing_config_repeats_and_removes_only_mailx() {
        let dir = tempfile::tempdir().unwrap();
        let locations = locations(dir.path());
        let path = locations.codex.join("config.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            "# keep me\nmodel = \"example\"\n[mcp_servers.other]\ncommand = \"other\"\n",
        )
        .unwrap();
        assert_eq!(
            modify(McpAgentId::Codex, &config(), &locations, false, false),
            Ok("configured")
        );
        let first = std::fs::read_to_string(&path).unwrap();
        assert!(first.contains("# keep me"));
        assert_eq!(
            modify(McpAgentId::Codex, &config(), &locations, false, false),
            Ok("unchanged")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), first);
        assert_eq!(
            modify(McpAgentId::Codex, &config(), &locations, false, true),
            Ok("removed")
        );
        assert!(
            std::fs::read_to_string(path)
                .unwrap()
                .contains("[mcp_servers.other]")
        );
    }
    #[test]
    fn repairs_remote_and_mixed_entries_without_losing_tool_policies() {
        let dir = tempfile::tempdir().unwrap();
        let locations = locations(dir.path());
        let path = locations.codex.join("config.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path,"[mcp_servers.mailx]\nurl = \"https://old.example/mcp\"\nbearer_token_env_var = \"TOKEN\"\ntool_timeout_sec = 120\n").unwrap();
        assert_eq!(
            describe(McpAgentId::Codex, &config(), Some(&locations)).configuration,
            "conflict"
        );
        assert_eq!(
            modify(McpAgentId::Codex, &config(), &locations, false, false),
            Ok("conflict")
        );
        assert_eq!(
            modify(McpAgentId::Codex, &config(), &locations, true, false),
            Ok("configured")
        );
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(!out.contains("url ="));
        assert!(!out.contains("bearer_token_env_var"));
        assert!(out.contains("tool_timeout_sec = 120"));
        assert_eq!(
            describe(McpAgentId::Codex, &config(), Some(&locations)).configuration,
            "configured"
        );
        // A matching command/args must not mask a remaining remote URL.
        let mixed = out + "url = \"https://old.example/mcp\"\n";
        std::fs::write(&path, mixed).unwrap();
        assert_eq!(
            describe(McpAgentId::Codex, &config(), Some(&locations)).configuration,
            "conflict"
        );
        let path = locations.opencode.join("opencode.jsonc");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path,r#"{ // keep
            "mcp":{"servers":{"mailx":{"type":"remote","url":"https://old.example/mcp","headers":{"Authorization":"secret"},"timeout":{"execution":120000}}}}
        }"#).unwrap();
        assert_eq!(
            modify(McpAgentId::OpenCode, &config(), &locations, true, false),
            Ok("configured")
        );
        let out = std::fs::read_to_string(&path).unwrap();
        let saved = current_entry(McpAgentId::OpenCode, &out).unwrap().unwrap();
        assert_eq!(saved["type"], "local");
        assert!(saved.get("url").is_none());
        assert!(saved.get("headers").is_none());
        assert_eq!(saved["timeout"]["execution"], 120000);
        assert!(out.contains("// keep"));
        assert_eq!(
            describe(McpAgentId::OpenCode, &config(), Some(&locations)).configuration,
            "configured"
        );
    }
    #[test]
    fn another_data_directory_requires_explicit_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let locations = locations(dir.path());
        let a = config();
        let mut b = config();
        b.args[1] = "/other/mcp.json".into();
        assert_eq!(
            modify(McpAgentId::Codex, &a, &locations, false, false),
            Ok("configured")
        );
        assert_eq!(
            modify(McpAgentId::Codex, &b, &locations, false, false),
            Ok("conflict")
        );
        assert_eq!(
            modify(McpAgentId::Codex, &b, &locations, false, true),
            Ok("conflict")
        );
        assert_eq!(
            modify(McpAgentId::Codex, &b, &locations, true, false),
            Ok("configured")
        );
    }
    #[test]
    fn opencode_keeps_legacy_and_modern_layouts() {
        assert_eq!(
            json_path(
                McpAgentId::OpenCode,
                "{\"mcp\":{\"other\":{\"type\":\"local\"}}}"
            )
            .unwrap(),
            vec!["mcp", "mailx"]
        );
        assert_eq!(
            json_path(McpAgentId::OpenCode, "{\"mcp\":{\"servers\":{}}}").unwrap(),
            vec!["mcp", "servers", "mailx"]
        );
        assert_eq!(
            json_path(McpAgentId::OpenCode, "{}").unwrap(),
            vec!["mcp", "servers", "mailx"]
        );
    }
    #[test]
    fn official_import_links_roundtrip_unicode_paths_as_single_server() {
        for id in [McpAgentId::Cursor, McpAgentId::Trae] {
            let link = import_url(id, &config()).unwrap();
            let url = tauri::Url::parse(&link).unwrap();
            let encoded = url
                .query_pairs()
                .find(|(key, _)| key == "config")
                .unwrap()
                .1
                .into_owned();
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .unwrap();
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(value, entry(&config(), id));
            assert!(value.get("mcpServers").is_none());
        }
    }
    #[test]
    fn broken_client_configuration_does_not_hide_other_clients() {
        let dir = tempfile::tempdir().unwrap();
        let locations = locations(dir.path());
        let path = locations.codex.join("config.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "invalid = [").unwrap();
        assert_eq!(
            describe(McpAgentId::Codex, &config(), Some(&locations)).configuration,
            "unavailable"
        );
        assert_eq!(
            describe(McpAgentId::Cursor, &config(), Some(&locations)).configuration,
            "missing"
        );
        assert_eq!(
            describe(McpAgentId::DeepseekHarness, &config(), Some(&locations)).method,
            "manual"
        );
        assert!(
            snippet(McpAgentId::DeepseekHarness, &config(), None)
                .contains("@deepseek-ai/dsh-mcp-client")
        );
    }
}
