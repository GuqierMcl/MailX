//! Lossless edits of a single MCP entry; never rewrite unrelated user settings.
use jsonc_parser::{
    ParseOptions,
    cst::{CstInputValue, CstObject, CstRootNode},
};
use serde_json::Value;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::Path,
};
use toml_edit::{DocumentMut, Item, Table, value};

pub(super) const REMOTE_FIELDS: &[&str] = &[
    "url",
    "serverUrl",
    "httpUrl",
    "headers",
    "http_headers",
    "env_http_headers",
    "http_headers_helper",
    "bearer_token_env_var",
    "bearerToken",
    "oauth",
    "auth",
];
pub(super) const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;

pub(super) fn read(path: &Path) -> Result<Option<String>, ()> {
    match fs::symlink_metadata(path) {
        Ok(meta)
            if meta.is_file()
                && !meta.file_type().is_symlink()
                && meta.len() <= MAX_CONFIG_BYTES => {}
        Ok(_) => return Err(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(()),
    }
    let mut text = String::new();
    File::open(path)
        .map_err(|_| ())?
        .take(MAX_CONFIG_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|_| ())?;
    if text.len() as u64 > MAX_CONFIG_BYTES {
        return Err(());
    }
    Ok(Some(text))
}

fn input(value: &Value) -> CstInputValue {
    match value {
        Value::Null => CstInputValue::Null,
        Value::Bool(v) => CstInputValue::Bool(*v),
        Value::Number(v) => CstInputValue::Number(v.to_string()),
        Value::String(v) => CstInputValue::String(v.clone()),
        Value::Array(v) => CstInputValue::Array(v.iter().map(input).collect()),
        Value::Object(v) => {
            CstInputValue::Object(v.iter().map(|(k, v)| (k.clone(), input(v))).collect())
        }
    }
}

fn json_root(text: &str) -> Result<CstRootNode, ()> {
    let options = ParseOptions {
        allow_comments: true,
        allow_trailing_commas: true,
        allow_loose_object_property_names: false,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
        allow_bare_decimal_point_numbers: false,
        allow_non_finite_numbers: false,
        allow_extended_string_escapes: false,
    };
    let root = CstRootNode::parse(text, &options).map_err(|_| ())?;
    let object = root.object_value().ok_or(())?;
    reject_duplicate_keys(&object)?;
    Ok(root)
}

fn reject_duplicate_keys(object: &CstObject) -> Result<(), ()> {
    let mut names = std::collections::HashSet::new();
    for prop in object.properties() {
        let name = prop.name().ok_or(())?.decoded_value().map_err(|_| ())?;
        if !names.insert(name) {
            return Err(());
        }
        if let Some(child) = prop.object_value() {
            reject_duplicate_keys(&child)?;
        }
    }
    Ok(())
}

pub(super) fn json_entry(text: &str, path: &[&str]) -> Result<Option<Value>, ()> {
    let root = json_root(text)?;
    let mut node = root.object_value().ok_or(())?;
    for key in &path[..path.len() - 1] {
        match node.get(key) {
            None => return Ok(None),
            Some(prop) => node = prop.object_value().ok_or(())?,
        }
    }
    Ok(node
        .get(path[path.len() - 1])
        .and_then(|prop| prop.value())
        .and_then(|value| value.to_serde_value()))
}

pub(super) fn edit_json(text: &str, path: &[&str], entry: Option<&Value>) -> Result<String, ()> {
    let root = json_root(text)?;
    let mut node = root.object_value().ok_or(())?;
    for key in &path[..path.len() - 1] {
        node = node.object_value_or_create(key).ok_or(())?;
    }
    let key = path[path.len() - 1];
    if let Some(entry) = entry {
        // Preserve settings on the MailX entry, including client-specific timeouts.
        if let Some(prop) = node.get(key) {
            let target = prop.object_value().ok_or(())?;
            for key in REMOTE_FIELDS {
                if let Some(prop) = target.get(key) {
                    prop.remove();
                }
            }
            for key in ["type", "transport"] {
                if entry.get(key).is_none()
                    && let Some(prop) = target.get(key)
                {
                    prop.remove();
                }
            }
            for (key, value) in entry.as_object().ok_or(())? {
                if let Some(prop) = target.get(key) {
                    prop.set_value(input(value));
                } else {
                    target.append(key, input(value));
                }
            }
        } else {
            node.append(key, input(entry));
        }
    } else if let Some(prop) = node.get(key) {
        prop.remove();
    }
    Ok(root.to_string())
}

pub(super) fn toml_entry(text: &str) -> Result<Option<Value>, ()> {
    let doc = text.parse::<DocumentMut>().map_err(|_| ())?;
    let Some(servers) = doc.get("mcp_servers") else {
        return Ok(None);
    };
    let servers = servers.as_table_like().ok_or(())?;
    let Some(entry) = servers.get("mailx") else {
        return Ok(None);
    };
    let entry = entry.as_table_like().ok_or(())?;
    let mut out = serde_json::Map::new();
    for key in ["command", "args", "enabled"] {
        if let Some(item) = entry.get(key) {
            let v = if key == "args" {
                Value::Array(
                    item.as_array()
                        .ok_or(())?
                        .iter()
                        .map(|v| v.as_str().map(|v| Value::String(v.into())).ok_or(()))
                        .collect::<Result<Vec<_>, _>>()?,
                )
            } else if key == "enabled" {
                Value::Bool(item.as_bool().ok_or(())?)
            } else {
                Value::String(item.as_str().ok_or(())?.into())
            };
            out.insert(key.into(), v);
        }
    }
    for key in REMOTE_FIELDS.iter().copied().chain(["type", "transport"]) {
        if let Some(item) = entry.get(key) {
            // Presence is enough to reject a remote/mixed transport. Never
            // serialize credentials from unrelated fields into a UI result.
            out.insert(
                key.into(),
                item.as_str()
                    .map_or(Value::Bool(true), |s| Value::String(s.into())),
            );
        }
    }
    Ok(Some(Value::Object(out)))
}

pub(super) fn edit_toml(text: &str, entry: Option<&Value>) -> Result<String, ()> {
    let mut doc = text.parse::<DocumentMut>().map_err(|_| ())?;
    if doc.get("mcp_servers").is_none() {
        doc["mcp_servers"] = Item::Table(Table::new());
    }
    let servers = doc
        .get_mut("mcp_servers")
        .and_then(Item::as_table_like_mut)
        .ok_or(())?;
    if let Some(entry) = entry {
        if servers.get("mailx").is_none() {
            servers.insert("mailx", Item::Table(Table::new()));
        }
        let target = servers
            .get_mut("mailx")
            .and_then(Item::as_table_like_mut)
            .ok_or(())?;
        for key in REMOTE_FIELDS.iter().copied().chain(["type", "transport"]) {
            target.remove(key);
        }
        target.insert("command", value(entry["command"].as_str().ok_or(())?));
        let mut args = toml_edit::Array::new();
        for arg in entry["args"].as_array().ok_or(())? {
            args.push(arg.as_str().ok_or(())?);
        }
        target.insert("args", value(args));
        // Repair a disabled entry only on the explicit replace path.
        if target.get("enabled").is_some() {
            target.insert("enabled", value(true));
        }
    } else {
        servers.remove("mailx");
    }
    Ok(doc.to_string())
}

/// Publishes complete files without overwriting a competing save. An existing
/// configuration is moved to a unique backup, then its actual contents are
/// checked before the new file is linked into the now-vacant destination.
///
/// There is a short interval with no destination between rename and link.
/// Ordinary failures restore the backup only if that destination is still
/// absent. A crash in that interval can require manual recovery from the
/// retained backup; this is not an atomic compare-and-swap or a merge editor.
pub(super) fn write(path: &Path, before: Option<&str>, after: &str) -> Result<(), ()> {
    write_with_hook(path, before, after, |_, _, _| Ok(()))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WritePhase {
    Checked,
    Moved,
    Publishing,
    Restoring,
}

/// Retains the actual file displaced by rename, including a racing editor's
/// version. Linking never replaces a path another writer has since created.
struct RestoreBackup<'a, F: FnMut(WritePhase, &Path, Option<&Path>) -> Result<(), ()>> {
    path: &'a Path,
    backup: std::path::PathBuf,
    armed: bool,
    hook: &'a mut F,
}

impl<F: FnMut(WritePhase, &Path, Option<&Path>) -> Result<(), ()>> Drop for RestoreBackup<'_, F> {
    fn drop(&mut self) {
        if self.armed {
            let _ = (self.hook)(WritePhase::Restoring, self.path, Some(&self.backup));
            // AlreadyExists deliberately preserves a competing new file.
            // The backup remains available even when restoration fails.
            let _ = fs::hard_link(&self.backup, self.path);
        }
    }
}

fn write_with_hook(
    path: &Path,
    before: Option<&str>,
    after: &str,
    mut hook: impl FnMut(WritePhase, &Path, Option<&Path>) -> Result<(), ()>,
) -> Result<(), ()> {
    let parent = path.parent().ok_or(())?;
    if !parent.exists() {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(parent).map_err(|_| ())?;
    }
    let nonce = uuid::Uuid::new_v4();
    let temporary = parent.join(format!(".mailx-{nonce}.tmp"));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options.open(&temporary).map_err(|_| ())?;
        file.write_all(after.as_bytes()).map_err(|_| ())?;
        file.sync_all().map_err(|_| ())?;
        // Close before publishing, including on Windows.
        drop(file);
        if read(path)?.as_deref() != before {
            return Err(());
        }
        hook(WritePhase::Checked, path, None)?;
        if before.is_none() {
            // Unlike rename, a hard link fails atomically if someone creates
            // the previously absent destination after the check above.
            return fs::hard_link(&temporary, path).map_err(|_| ());
        }
        // Refuse filesystems without hard links before displacing an existing
        // configuration; there is then no safe no-clobber restore primitive.
        let probe = parent.join(format!(".mailx-{nonce}.link"));
        fs::hard_link(&temporary, &probe).map_err(|_| ())?;
        fs::remove_file(&probe).map_err(|_| ())?;
        let backup = parent.join(format!(
            "{}.mailx-{nonce}.bak",
            path.file_name().ok_or(())?.to_string_lossy()
        ));
        // The UUID destination is never shared with another MailX writer.
        fs::rename(path, &backup).map_err(|_| ())?;
        let mut restore = RestoreBackup {
            path,
            backup,
            armed: true,
            hook: &mut hook,
        };
        (restore.hook)(WritePhase::Moved, path, Some(&restore.backup))?;
        let actual = read(&restore.backup)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&restore.backup, fs::Permissions::from_mode(0o600))
                .map_err(|_| ())?;
        }
        if actual.as_deref() != before {
            return Err(());
        }
        (restore.hook)(WritePhase::Publishing, path, Some(&restore.backup))?;
        fs::hard_link(&temporary, path).map_err(|_| ())?;
        restore.armed = false;
        Ok(())
    })();
    // Successful publication leaves a second link to the staging inode; all
    // failure paths also discard staging without touching any competing save.
    let _ = fs::remove_file(temporary);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn jsonc_preserves_comments_other_servers_and_unknown_settings() {
        let text = "{\n // important\n \"model\": \"x\",\n \"mcp\": {\"servers\": {\"other\": {\"command\": [\"other\"]}, \"mailx\": {\"timeout\": 123, \"type\": \"local\", \"command\": [\"old\"]}}},\n}\n";
        let out = edit_json(
            text,
            &["mcp", "servers", "mailx"],
            Some(&json!({"type":"local","command":["new"]})),
        )
        .unwrap();
        assert!(out.contains("// important"));
        assert!(out.contains("\"timeout\": 123"));
        assert_eq!(
            json_entry(&out, &["mcp", "servers", "other"]).unwrap(),
            Some(json!({"command":["other"]}))
        );
        let removed = edit_json(&out, &["mcp", "servers", "mailx"], None).unwrap();
        assert!(
            json_entry(&removed, &["mcp", "servers", "mailx"])
                .unwrap()
                .is_none()
        );
        assert!(removed.contains("// important"));
    }
    #[test]
    fn invalid_and_duplicate_json_never_get_replaced() {
        for text in [
            "[]",
            "{broken",
            "{\"mcpServers\":{},\"mcpServers\":{}}",
            "{\"mcpServers\": 2}",
        ] {
            assert!(edit_json(text, &["mcpServers", "mailx"], Some(&json!({}))).is_err());
        }
    }
    #[test]
    fn toml_preserves_comments_credentials_and_other_servers() {
        let text = "# private user settings\nmodel = \"example\"\n[mcp_servers.other]\ncommand = \"other\"\n[mcp_servers.mailx]\ncommand = \"old\"\nargs = []\ntool_timeout_sec = 120\n";
        let out = edit_toml(
            text,
            Some(&json!({"command":"/new path/mailx","args":["--discovery","/数据/data"]})),
        )
        .unwrap();
        assert!(out.contains("# private user settings"));
        assert!(out.contains("tool_timeout_sec = 120"));
        assert_eq!(toml_entry(&out).unwrap().unwrap()["args"][1], "/数据/data");
        let removed = edit_toml(&out, None).unwrap();
        assert!(removed.contains("[mcp_servers.other]"));
        assert!(toml_entry(&removed).unwrap().is_none());
    }
    #[test]
    fn writes_backup_and_refuses_late_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, "old").unwrap();
        write(&path, Some("old"), "new").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        assert!(
            fs::read_dir(dir.path())
                .unwrap()
                .filter_map(Result::ok)
                .any(|e| e.file_name().to_string_lossy().ends_with(".bak"))
        );
        assert!(write(&path, Some("old"), "overwrite").is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "new");
    }
    fn backup_contents(directory: &Path) -> Vec<String> {
        fs::read_dir(directory)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".bak"))
            .map(|entry| fs::read_to_string(entry.path()).unwrap())
            .collect()
    }

    #[test]
    fn new_configuration_never_overwrites_a_racing_creation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let result = write_with_hook(&path, None, "mailx", |phase, path, _| {
            if phase == WritePhase::Checked {
                fs::write(path, "created by editor").unwrap();
            }
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "created by editor");
        assert!(backup_contents(dir.path()).is_empty());
    }

    #[test]
    fn existing_configuration_retains_the_actual_racing_save() {
        for atomic in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.json");
            fs::write(&path, "before").unwrap();
            let result = write_with_hook(&path, Some("before"), "mailx", |phase, path, _| {
                if phase == WritePhase::Checked {
                    if atomic {
                        let pending = dir.path().join("editor.tmp");
                        fs::write(&pending, "saved by editor").unwrap();
                        fs::rename(pending, path).unwrap();
                    } else {
                        fs::write(path, "saved by editor").unwrap();
                    }
                }
                Ok(())
            });
            assert!(result.is_err());
            assert_eq!(fs::read_to_string(path).unwrap(), "saved by editor");
            assert_eq!(backup_contents(dir.path()), vec!["saved by editor"]);
        }
    }

    #[test]
    fn competing_creation_after_backup_is_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, "before").unwrap();
        let result = write_with_hook(&path, Some("before"), "mailx", |phase, path, _| {
            if phase == WritePhase::Moved {
                fs::write(path, "created while vacant").unwrap();
            }
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "created while vacant");
        assert_eq!(backup_contents(dir.path()), vec!["before"]);
    }

    #[test]
    fn publication_failure_restores_without_discarding_the_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, "before").unwrap();
        let result = write_with_hook(&path, Some("before"), "mailx", |phase, _, _| {
            if phase == WritePhase::Publishing {
                Err(())
            } else {
                Ok(())
            }
        });
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "before");
        assert_eq!(backup_contents(dir.path()), vec!["before"]);
    }

    #[test]
    fn restore_never_overwrites_a_competing_creation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        fs::write(&path, "before").unwrap();
        let result = write_with_hook(&path, Some("before"), "mailx", |phase, path, _| {
            if phase == WritePhase::Publishing {
                return Err(());
            }
            if phase == WritePhase::Restoring {
                fs::write(path, "created before restore").unwrap();
            }
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), "created before restore");
        assert_eq!(backup_contents(dir.path()), vec!["before"]);
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlink_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        let link = dir.path().join("link");
        fs::write(&target, "private").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(read(&link).is_err());
        assert!(write(&link, Some("private"), "bad").is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), "private");
    }
}
