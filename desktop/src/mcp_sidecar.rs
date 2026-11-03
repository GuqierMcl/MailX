//! Where the `mailx-mcp` bridge lives and what a client must run to use it.
//!
//! Tauri places `bundle.externalBin` files next to the main executable (macOS
//! `Contents/MacOS/`, the Windows install directory, `/usr/bin` for deb / rpm),
//! and `tauri dev` copies the same file next to the debug binary. An AppImage
//! is different: its mount point changes on every start, so a path written
//! into a client's config would go stale. There the bridge is copied to a
//! stable per-user location and that copy is handed out instead.

use std::path::{Path, PathBuf};

use mailx_core::{AppError, ErrorCode};
use mailx_mcp_server::discovery_path;
use serde::Serialize;

#[cfg(windows)]
const BRIDGE_FILE: &str = "mailx-mcp.exe";
#[cfg(not(windows))]
const BRIDGE_FILE: &str = "mailx-mcp";

/// Directory below the data directory that holds the AppImage copy of the bridge.
const APPIMAGE_DIR: [&str; 2] = ["mcp", "bin"];

/// What the settings page puts into a client's `mcpServers` entry.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct McpClientConfig {
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    /// The same two paths for showing only; `command` / `args` are what gets copied.
    pub(crate) display: McpClientDisplay,
}

/// The bridge and discovery file paths with the home directory written as `~`.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct McpClientDisplay {
    pub(crate) command: String,
    pub(crate) discovery: String,
}

/// `path` with its leading `home` directory replaced by `~`, for display.
///
/// Paths outside `home` come back unchanged. The match is on whole components, so
/// `/Users/ab` is not inside `/Users/a`. A Windows-style home (backslashes or a drive
/// letter) matches case-insensitively and treats `/` like `\`; the rest of `path`
/// keeps its own separators.
fn shorten_home(path: &str, home: &str) -> String {
    let windows = home.contains('\\') || home.as_bytes().get(1) == Some(&b':');
    let is_separator = |c: char| c == '/' || (windows && c == '\\');
    let home = home.trim_end_matches(is_separator);
    if home.is_empty() {
        return path.to_owned();
    }
    let same = |a: char, b: char| {
        if windows {
            (is_separator(a) && is_separator(b)) || a.to_lowercase().eq(b.to_lowercase())
        } else {
            a == b
        }
    };
    let mut rest = path.chars();
    for expected in home.chars() {
        match rest.next() {
            Some(actual) if same(actual, expected) => {}
            _ => return path.to_owned(),
        }
    }
    let rest = rest.as_str();
    if rest.is_empty() || rest.starts_with(is_separator) {
        format!("~{rest}")
    } else {
        path.to_owned()
    }
}

/// The bridge beside `exe`, or its stable copy under `data_dir` for an AppImage.
pub(crate) fn locate_bridge(
    exe: &Path,
    data_dir: &Path,
    appimage: bool,
) -> Result<PathBuf, AppError> {
    let bundled = exe
        .parent()
        .map(|dir| dir.join(BRIDGE_FILE))
        // Tauri's build script leaves an empty placeholder when the real bridge is not built.
        .filter(|path| {
            path.metadata()
                .is_ok_and(|meta| meta.is_file() && meta.len() > 0)
        })
        .ok_or_else(|| AppError::new(ErrorCode::Internal))?;
    if appimage {
        stable_copy(&bundled, data_dir)
    } else {
        Ok(bundled)
    }
}

/// Copies `bundled` to `<data_dir>/mcp/bin/mailx-mcp` unless an identical copy is there.
fn stable_copy(bundled: &Path, data_dir: &Path) -> Result<PathBuf, AppError> {
    let internal = |_| AppError::new(ErrorCode::Internal);
    let directory = APPIMAGE_DIR
        .iter()
        .fold(data_dir.to_path_buf(), |path, part| path.join(part));
    create_private_directories(&directory).map_err(internal)?;
    let target = directory.join(BRIDGE_FILE);
    let current = std::fs::read(bundled).map_err(internal)?;
    if std::fs::read(&target).is_ok_and(|existing| existing == current) {
        return Ok(target);
    }
    // Write beside the target and rename so a client that is starting the bridge
    // right now never sees a half-written executable.
    let staging = directory.join(format!("{BRIDGE_FILE}.tmp"));
    std::fs::write(&staging, &current).map_err(internal)?;
    set_executable(&staging).map_err(internal)?;
    std::fs::rename(&staging, &target).map_err(internal)?;
    Ok(target)
}

#[cfg(unix)]
fn create_private_directories(directory: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)?;
    // `mcp/` and `mcp/bin/` stay private even when an older run created them wider.
    for path in directory.ancestors().take(APPIMAGE_DIR.len()) {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn create_private_directories(directory: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(directory)
}

#[cfg(unix)]
fn set_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Command and arguments for a client. `--discovery` is always present: the data
/// directory can be moved (`MAILX_DATA_DIR`, storage location setting) and the
/// bridge's built-in default would then point at the wrong place.
pub(crate) fn client_config(
    exe: &Path,
    data_dir: &Path,
    appimage: bool,
) -> Result<McpClientConfig, AppError> {
    // Clients spawn these paths as they are, and many cannot handle a `\\?\` prefix.
    let data_dir = dunce::simplified(data_dir);
    let bridge = locate_bridge(exe, data_dir, appimage)?;
    let text = |path: &Path| {
        path.to_str()
            .map(str::to_owned)
            .ok_or_else(|| AppError::new(ErrorCode::Internal))
    };
    let command = text(dunce::simplified(&bridge))?;
    let discovery = text(dunce::simplified(&discovery_path(data_dir)))?;
    let display = |path: &str| match std::env::home_dir() {
        Some(home) => home
            .to_str()
            .map_or_else(|| path.to_owned(), |home| shorten_home(path, home)),
        None => path.to_owned(),
    };
    Ok(McpClientConfig {
        display: McpClientDisplay {
            command: display(&command),
            discovery: display(&discovery),
        },
        command,
        args: vec!["--discovery".to_owned(), discovery],
    })
}

/// [`client_config`] for the running process.
pub(crate) fn current_client_config(data_dir: &Path) -> Result<McpClientConfig, AppError> {
    let exe = std::env::current_exe().map_err(|_| AppError::new(ErrorCode::Internal))?;
    let exe = dunce::canonicalize(exe).map_err(|_| AppError::new(ErrorCode::Internal))?;
    let appimage = std::env::var_os("APPIMAGE").is_some_and(|value| !value.is_empty());
    client_config(&exe, data_dir, appimage)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app_dir_with_bridge(contents: &[u8]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("mailx-desktop");
        std::fs::write(&exe, b"app").unwrap();
        std::fs::write(dir.path().join(BRIDGE_FILE), contents).unwrap();
        (dir, exe)
    }

    #[test]
    fn bridge_next_to_the_executable_is_used_in_place() {
        let (dir, exe) = app_dir_with_bridge(b"bridge");
        let data = tempfile::tempdir().unwrap();
        let found = locate_bridge(&exe, data.path(), false).unwrap();
        assert_eq!(found, dir.path().join(BRIDGE_FILE));
        assert!(!data.path().join("mcp").exists());
    }

    #[test]
    fn missing_bridge_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("mailx-desktop");
        std::fs::write(&exe, b"app").unwrap();
        assert!(locate_bridge(&exe, dir.path(), false).is_err());
    }

    #[test]
    fn empty_placeholder_bridge_counts_as_missing() {
        let (dir, exe) = app_dir_with_bridge(b"");
        assert!(locate_bridge(&exe, dir.path(), false).is_err());
    }

    #[test]
    fn appimage_copies_the_bridge_to_a_stable_place() {
        let (_dir, exe) = app_dir_with_bridge(b"v1");
        let data = tempfile::tempdir().unwrap();
        let found = locate_bridge(&exe, data.path(), true).unwrap();
        assert_eq!(found, data.path().join("mcp").join("bin").join(BRIDGE_FILE));
        assert_eq!(std::fs::read(&found).unwrap(), b"v1");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&found), 0o755);
            assert_eq!(mode(found.parent().unwrap()), 0o700);
            assert_eq!(mode(found.parent().unwrap().parent().unwrap()), 0o700);
        }
    }

    #[test]
    fn appimage_copy_is_replaced_when_the_bundled_bridge_changes() {
        let (dir, exe) = app_dir_with_bridge(b"v1");
        let data = tempfile::tempdir().unwrap();
        let first = locate_bridge(&exe, data.path(), true).unwrap();
        std::fs::write(dir.path().join(BRIDGE_FILE), b"v2").unwrap();
        let second = locate_bridge(&exe, data.path(), true).unwrap();
        assert_eq!(first, second);
        assert_eq!(std::fs::read(&second).unwrap(), b"v2");
        assert!(!second.with_file_name(format!("{BRIDGE_FILE}.tmp")).exists());
    }

    #[test]
    fn client_config_always_names_the_discovery_file() {
        let (dir, exe) = app_dir_with_bridge(b"bridge");
        let data = tempfile::tempdir().unwrap();
        let config = client_config(&exe, data.path(), false).unwrap();
        assert_eq!(
            config.command,
            dir.path().join(BRIDGE_FILE).to_str().unwrap()
        );
        assert_eq!(
            config.args,
            vec![
                "--discovery".to_owned(),
                discovery_path(data.path()).to_str().unwrap().to_owned()
            ]
        );
    }

    #[test]
    fn client_config_displays_the_same_paths_when_outside_home() {
        // Temp directories sit outside the home directory, so nothing is shortened.
        let (_dir, exe) = app_dir_with_bridge(b"bridge");
        let data = tempfile::tempdir().unwrap();
        let config = client_config(&exe, data.path(), false).unwrap();
        assert_eq!(config.display.command, config.command);
        assert_eq!(config.display.discovery, config.args[1]);
    }

    #[test]
    fn shorten_home_replaces_the_home_prefix() {
        let home = "/Users/a";
        assert_eq!(
            shorten_home("/Users/a/Library/MailX/mailx-mcp", home),
            "~/Library/MailX/mailx-mcp"
        );
        assert_eq!(shorten_home("/Users/a", home), "~");
        assert_eq!(shorten_home("/Users/a/", "/Users/a/"), "~/");
        assert_eq!(shorten_home("/Users/a/x", "/Users/a/"), "~/x");
    }

    #[test]
    fn shorten_home_leaves_other_paths_alone() {
        let home = "/Users/a";
        for path in [
            "/Applications/MailX.app/Contents/MacOS/mailx-mcp",
            "/usr/bin/mailx-mcp",
            "/Users/ab/x",
            "/users/a/x",
            "/Users",
        ] {
            assert_eq!(shorten_home(path, home), path);
        }
        assert_eq!(shorten_home("/x", "/"), "/x");
        assert_eq!(shorten_home("/x", ""), "/x");
    }

    #[test]
    fn shorten_home_matches_windows_paths_case_insensitively() {
        let home = r"C:\Users\Guqier";
        assert_eq!(
            shorten_home(r"C:\Users\Guqier\AppData\Local\MailX\mailx-mcp.exe", home),
            r"~\AppData\Local\MailX\mailx-mcp.exe"
        );
        assert_eq!(
            shorten_home(r"c:\users\GUQIER\AppData\x", home),
            r"~\AppData\x"
        );
        assert_eq!(shorten_home(r"C:\Users\Guqier", home), "~");
        assert_eq!(shorten_home("C:/Users/Guqier/x", home), "~/x");
        assert_eq!(
            shorten_home(r"C:\Users\Guqier2\x", home),
            r"C:\Users\Guqier2\x"
        );
        assert_eq!(
            shorten_home(r"C:\Program Files\MailX\mailx-mcp.exe", home),
            r"C:\Program Files\MailX\mailx-mcp.exe"
        );
    }

    #[test]
    fn normal_paths_are_unchanged_by_simplification() {
        let path = Path::new("/opt/MailX/mailx-mcp");
        assert_eq!(dunce::simplified(path), path);
    }

    #[cfg(windows)]
    #[test]
    fn windows_client_config_has_no_verbatim_prefix() {
        let (_dir, exe) = app_dir_with_bridge(b"bridge");
        let data = tempfile::tempdir().unwrap();
        // `fs::canonicalize` would return `\\?\C:\...` for these.
        let exe = dunce::canonicalize(exe).unwrap();
        let data_dir = std::fs::canonicalize(data.path()).unwrap();
        assert!(data_dir.to_str().unwrap().starts_with(r"\\?\"));
        let config = client_config(&exe, &data_dir, false).unwrap();
        for text in [&config.command, &config.args[1], &config.display.command] {
            assert!(!text.starts_with(r"\\?\"), "{text}");
        }
    }
}
