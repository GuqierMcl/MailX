//! Where MailX keeps its data and secrets. Both are named after the product, not
//! the application identifier, so a later identifier change moves neither the
//! database nor the stored passwords.
//!
//! Tauri still derives some directories from the identifier on its own: the
//! WebView storage and caches, and the window-state file in `app_config_dir()`.
//! Those hold nothing that cannot be rebuilt.
use std::path::{Path, PathBuf};

use mailx_core::{AppError, ErrorCode};

/// Folder under the platform's application data location (the parent of Tauri's
/// `app_data_dir()`: Application Support on macOS, `%APPDATA%` on Windows,
/// `~/.local/share` on Linux).
pub(crate) const DATA_DIR_NAME: &str = "MailX";

/// Service name of every credential in the system store.
pub(crate) const SECRET_SERVICE: &str = "MailX";

/// Names an alternative user-data-dir. Smoke tests and throwaway profiles set it
/// so they never touch the real data or the real credentials.
pub(crate) const DATA_DIR_OVERRIDE: &str = "MAILX_DATA_DIR";

/// Where the data lives and which credential service goes with it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Storage {
    pub(crate) data_dir: PathBuf,
    pub(crate) secret_service: String,
}

/// The storage for a Tauri `app_data_dir()` such as `<base>/<identifier>`.
///
/// `override_dir` is the value of [`DATA_DIR_OVERRIDE`]. It must be an absolute
/// path, and its credentials get their own service name (`MailX.<folder>`), so an
/// override can never read or overwrite the real passwords.
pub(crate) fn storage(
    app_data_dir: &Path,
    override_dir: Option<&std::ffi::OsStr>,
) -> Result<Storage, AppError> {
    let unavailable = || AppError::new(ErrorCode::StorageUnavailable);
    if let Some(dir) = override_dir.filter(|dir| !dir.is_empty()) {
        let data_dir = PathBuf::from(dir);
        let name = data_dir
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(unavailable)?;
        if !data_dir.is_absolute() {
            return Err(unavailable());
        }
        return Ok(Storage {
            secret_service: format!("{SECRET_SERVICE}.{name}"),
            data_dir,
        });
    }
    match app_data_dir.parent() {
        Some(base) if !base.as_os_str().is_empty() => Ok(Storage {
            data_dir: base.join(DATA_DIR_NAME),
            secret_service: SECRET_SERVICE.to_owned(),
        }),
        _ => Err(unavailable()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use tauri::Manager;

    fn default_for(identifier_dir: &str) -> Storage {
        storage(Path::new(identifier_dir), None).unwrap()
    }

    #[test]
    fn the_folder_is_named_after_the_product_whatever_the_identifier_is() {
        let old = default_for("/base/dev.mailx.desktop");
        let new = default_for("/base/com.mailxapp.desktop");
        assert_eq!(old.data_dir, Path::new("/base/MailX"));
        assert_eq!(new, old);
        assert_eq!(new.secret_service, "MailX");
    }

    #[test]
    fn a_path_without_a_parent_directory_is_refused() {
        for path in ["", "/", "identifier"] {
            let error = storage(Path::new(path), None).unwrap_err();
            assert_eq!(error.code, ErrorCode::StorageUnavailable, "{path:?}");
        }
    }

    #[test]
    fn it_sits_next_to_the_directory_tauri_would_have_used() {
        let app = tauri::test::mock_app();
        let tauri_dir = app.path().app_data_dir().unwrap();
        let ours = storage(&tauri_dir, None).unwrap();
        assert_eq!(ours.data_dir.parent(), tauri_dir.parent());
        assert_eq!(ours.data_dir.file_name().unwrap(), DATA_DIR_NAME);
    }

    #[test]
    fn an_override_replaces_the_directory_and_separates_the_credentials() {
        let base = std::env::temp_dir().join("mailx-smoke");
        let chosen = storage(Path::new("/base/x"), Some(base.as_os_str())).unwrap();
        assert_eq!(chosen.data_dir, base);
        assert_eq!(chosen.secret_service, "MailX.mailx-smoke");
        assert_ne!(chosen.secret_service, SECRET_SERVICE);
    }

    #[test]
    fn an_empty_override_is_ignored_but_a_relative_or_nameless_one_is_refused() {
        let ignored = storage(Path::new("/base/x"), Some(OsStr::new(""))).unwrap();
        assert_eq!(ignored.data_dir, Path::new("/base/MailX"));
        for bad in ["relative/dir", "smoke", "/"] {
            let error = storage(Path::new("/base/x"), Some(OsStr::new(bad))).unwrap_err();
            assert_eq!(error.code, ErrorCode::StorageUnavailable, "{bad:?}");
        }
    }
}
