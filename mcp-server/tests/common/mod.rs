//! Shared fixtures. Also included by `mcp-bridge/tests` via `#[path]`.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use mailx_core::MailxCore;
use mailx_core::events::CoreEvent;
use mailx_core::secrets::{SecretStore, SecretStoreError, SecretValue};
use mailx_core::tools::Tools;
use mailx_mcp_server::McpConfig;
use serde_json::json;

#[derive(Default)]
struct MemorySecrets(Mutex<HashMap<String, SecretValue>>);

impl SecretStore for MemorySecrets {
    fn get(&self, key: &str) -> Result<Option<SecretValue>, SecretStoreError> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn set(&self, key: &str, value: &SecretValue) -> Result<(), SecretStoreError> {
        self.0.lock().unwrap().insert(key.into(), value.clone());
        Ok(())
    }
    fn delete(&self, key: &str) -> Result<(), SecretStoreError> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

pub struct Fixture {
    pub dir: tempfile::TempDir,
    pub tools: Arc<Tools>,
    pub account_id: String,
}

impl Fixture {
    /// A real core on a fresh data directory with one account that points at
    /// a closed local port, so nothing ever touches the network.
    pub async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let core = MailxCore::open(
            dir.path(),
            Arc::new(MemorySecrets::default()),
            tokio::runtime::Handle::current(),
            Arc::new(|_: &CoreEvent| {}),
        )
        .unwrap();
        let account = core
            .create_account(
                serde_json::from_value(json!({
                    "requestId": "00000000-0000-4000-8000-000000000001",
                    "config": {
                        "displayName": "Test", "emailAddress": "test@example.test", "providerId": null,
                        "imap": {"host": "127.0.0.1", "port": 1, "tls": "implicit", "username": "test"},
                        "smtp": {"host": "127.0.0.1", "port": 1, "tls": "implicit", "username": "test"},
                        "authentication": {"kind": "password", "mode": "shared", "password": {"action": "replace", "value": "x"}},
                        "route": {"kind": "direct"}
                    }
                }))
                .unwrap(),
            )
            .unwrap();
        let default_account = account.id.clone();
        let tools = Arc::new(Tools::new(core, move || default_account.clone()));
        Self {
            dir,
            tools,
            account_id: account.id,
        }
    }

    pub fn data_dir(&self) -> &Path {
        self.dir.path()
    }

    pub fn config(&self) -> McpConfig {
        McpConfig {
            data_dir: self.dir.path().to_path_buf(),
            tools: Arc::clone(&self.tools),
        }
    }
}
