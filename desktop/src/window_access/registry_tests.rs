//! The four places a host command must be listed stay in step: the
//! `generate_handler!` registration, the Tauri `AppManifest` in `build.rs`,
//! the capability files, and this module's window allowlist. A command missing
//! from the manifest or a capability is rejected by Tauri's ACL in the real
//! app while mock-based frontend tests still pass.

use std::collections::BTreeSet;
use std::path::Path;

use super::{BOTH, COMPOSER_ONLY, MAIN_ONLY};

fn read(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn unique(kind: &str, names: Vec<String>) -> BTreeSet<String> {
    let set: BTreeSet<String> = names.iter().cloned().collect();
    assert_eq!(set.len(), names.len(), "{kind} lists a command twice");
    set
}

fn between<'a>(text: &'a str, start: &str, end: &str) -> &'a str {
    let from = text
        .find(start)
        .unwrap_or_else(|| panic!("`{start}` not found"))
        + start.len();
    let len = text[from..]
        .find(end)
        .unwrap_or_else(|| panic!("`{end}` not found after `{start}`"));
    &text[from..from + len]
}

/// `module::command,` entries of `generate_handler![ ... ]`.
fn handler_commands() -> BTreeSet<String> {
    let main = read("src/main.rs");
    let body = between(&main, "generate_handler![", "]");
    unique(
        "generate_handler!",
        body.split(',')
            .map(|entry| entry.trim().rsplit("::").next().unwrap_or("").to_owned())
            .filter(|name| !name.is_empty())
            .collect(),
    )
}

/// String literals of the `AppManifest::new().commands(&[ ... ])` call.
fn manifest_commands() -> BTreeSet<String> {
    let build = read("build.rs");
    let body = between(&build, ".commands(&[", "]");
    unique(
        "build.rs manifest",
        body.split('"')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect(),
    )
}

/// `allow-some-command` permissions of one capability file as `some_command`;
/// `core:` plugin permissions are not host commands.
fn capability_commands(file: &str) -> BTreeSet<String> {
    let json: serde_json::Value = serde_json::from_str(&read(file)).unwrap();
    unique(
        file,
        json["permissions"]
            .as_array()
            .unwrap_or_else(|| panic!("{file}: no permissions"))
            .iter()
            .filter_map(|p| p.as_str())
            .filter(|p| !p.contains(':'))
            .map(|p| {
                p.strip_prefix("allow-")
                    .unwrap_or_else(|| panic!("{file}: unexpected permission {p}"))
                    .replace('-', "_")
            })
            .collect(),
    )
}

fn set(names: &[&str]) -> BTreeSet<String> {
    unique("allowlist", names.iter().map(|n| (*n).to_owned()).collect())
}

#[test]
fn handler_manifest_capabilities_and_allowlist_agree() {
    let handler = handler_commands();
    let manifest = manifest_commands();
    let main_cap = capability_commands("capabilities/main.json");
    let composer_cap = capability_commands("capabilities/composer.json");
    let main_only = set(MAIN_ONLY);
    let both = set(BOTH);
    let composer_only = set(COMPOSER_ONLY);

    let allowlist: BTreeSet<String> = main_only
        .iter()
        .chain(&both)
        .chain(&composer_only)
        .cloned()
        .collect();
    assert_eq!(
        main_only.len() + both.len() + composer_only.len(),
        allowlist.len(),
        "a command is in more than one window allowlist table"
    );
    assert_eq!(
        handler, allowlist,
        "generate_handler! (left) vs window_access tables (right)"
    );
    assert_eq!(
        handler, manifest,
        "generate_handler! (left) vs build.rs AppManifest (right)"
    );
    assert_eq!(
        manifest,
        main_cap.union(&composer_cap).cloned().collect(),
        "build.rs AppManifest (left) vs the union of capabilities/*.json (right)"
    );
    let main_expected: BTreeSet<String> = main_only.union(&both).cloned().collect();
    assert_eq!(
        main_expected, main_cap,
        "window_access main + both (left) vs capabilities/main.json (right)"
    );
    let composer_expected: BTreeSet<String> = composer_only.union(&both).cloned().collect();
    assert_eq!(
        composer_expected, composer_cap,
        "window_access composer + both (left) vs capabilities/composer.json (right)"
    );
}
