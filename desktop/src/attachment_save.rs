//! Saves an attachment to a place the user picks in the system save dialog.
//! The webview never supplies a path: it names an attachment and nothing else.
//! The destination is whatever the native dialog returns, and the bytes are
//! written through a temporary file in the same folder so a failure never
//! leaves a half-written file under the chosen name.
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use mailx_core::{AppError, ErrorCode};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum SaveOutcome {
    Saved,
    /// The user closed the dialog.
    Cancelled,
}

fn storage_error() -> AppError {
    AppError::new(ErrorCode::StorageUnavailable)
}

/// Writes `bytes` to `destination`, replacing a regular file the user chose to
/// overwrite. A symbolic link at the destination is refused rather than
/// followed: the dialog shows the link's name, not where it leads.
pub(crate) fn write_atomically(destination: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let directory = destination.parent().ok_or_else(storage_error)?;
    let name = destination.file_name().ok_or_else(storage_error)?;
    match fs::symlink_metadata(destination) {
        Ok(metadata) if metadata.file_type().is_symlink() || metadata.is_dir() => {
            return Err(storage_error());
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(storage_error()),
    }
    let temporary: PathBuf = directory.join(format!(
        ".{}.{}.part",
        name.to_string_lossy(),
        Uuid::new_v4()
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, destination)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|_| storage_error())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn names(directory: &Path) -> Vec<String> {
        let mut found: Vec<_> = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        found.sort();
        found
    }

    #[test]
    fn writes_a_new_file_and_leaves_no_temporary_behind() {
        let directory = TempDir::new().unwrap();
        let destination = directory.path().join("报告.pdf");
        write_atomically(&destination, b"contents").unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"contents");
        assert_eq!(names(directory.path()), ["报告.pdf"]);
    }

    #[test]
    fn replaces_an_existing_regular_file_the_user_agreed_to_overwrite() {
        let directory = TempDir::new().unwrap();
        let destination = directory.path().join("a.txt");
        fs::write(&destination, b"old").unwrap();
        write_atomically(&destination, b"new").unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"new");
        assert_eq!(names(directory.path()), ["a.txt"]);
    }

    #[test]
    fn refuses_directories_and_missing_folders_without_leaving_files() {
        let directory = TempDir::new().unwrap();
        let folder = directory.path().join("taken");
        fs::create_dir(&folder).unwrap();
        assert!(write_atomically(&folder, b"x").is_err());
        assert!(write_atomically(&directory.path().join("no-such").join("a.txt"), b"x").is_err());
        assert!(write_atomically(Path::new("/"), b"x").is_err());
        assert_eq!(names(directory.path()), ["taken"]);
    }

    #[cfg(unix)]
    #[test]
    fn never_writes_through_a_symbolic_link() {
        let directory = TempDir::new().unwrap();
        let target = directory.path().join("precious.txt");
        fs::write(&target, b"keep").unwrap();
        let link = directory.path().join("link.txt");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(write_atomically(&link, b"overwritten").is_err());
        assert_eq!(fs::read(&target).unwrap(), b"keep");
        let dangling = directory.path().join("dangling.txt");
        std::os::unix::fs::symlink(directory.path().join("absent"), &dangling).unwrap();
        assert!(write_atomically(&dangling, b"x").is_err());
        assert!(!directory.path().join("absent").exists());
    }
}
