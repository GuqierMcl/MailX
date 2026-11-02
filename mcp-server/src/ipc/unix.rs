//! Unix domain socket endpoint.
//!
//! Security model: the directory holding the socket is created 0700 and the
//! socket itself is chmod 0600, so only the owning user can reach it. The
//! window between `bind` and `chmod` is closed by the 0700 directory.

use std::fs::{self, DirBuilder, Permissions};
use std::io;
use std::os::unix::fs::{
    DirBuilderExt as _, FileTypeExt as _, MetadataExt as _, PermissionsExt as _,
};
use std::path::{Path, PathBuf};

use tokio::net::{UnixListener, UnixStream};

use super::{Stream, failure_code};
use crate::endpoint::{self, MAX_SOCKET_PATH_BYTES};
use crate::status::McpFailureCode;

pub(crate) struct Listener {
    listener: UnixListener,
    path: PathBuf,
}

impl Listener {
    pub(crate) async fn bind(data_dir: &Path, _instance_id: &str) -> Result<Self, McpFailureCode> {
        let path = prepare_path(data_dir).map_err(|error| failure_code(&error))?;
        remove_stale_socket(&path)
            .await
            .map_err(|error| failure_code(&error))?;
        let listener = UnixListener::bind(&path).map_err(|error| failure_code(&error))?;
        let listener = Self { listener, path };
        // Drop removes the socket file again if this chmod fails.
        fs::set_permissions(&listener.path, Permissions::from_mode(0o600))
            .map_err(|error| failure_code(&error))?;
        Ok(listener)
    }

    pub(crate) fn endpoint(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }

    pub(crate) async fn accept(&mut self) -> io::Result<Stream> {
        let (stream, _) = self.listener.accept().await?;
        Ok(Box::new(stream))
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Creates the socket directory (0700) and returns the final socket path,
/// moving to the per-user fallback directory when the preferred path is too
/// long. The data directory itself is created if missing but never chmod-ed.
fn prepare_path(data_dir: &Path) -> io::Result<PathBuf> {
    DirBuilder::new().recursive(true).create(data_dir)?;
    let uid = fs::metadata(data_dir)?.uid();
    let fallback = endpoint::fallback_root(uid);
    let (path, moved) = endpoint::choose_socket_path(data_dir, &fallback, MAX_SOCKET_PATH_BYTES);
    let dir = if moved {
        fallback.dir
    } else {
        data_dir.join(endpoint::SOCKET_DIR_NAME)
    };
    ensure_private_dir(&dir, uid)?;
    Ok(path)
}

/// Creates `dir` with mode 0700 if missing, then requires a real directory
/// (not a symlink) owned by `uid`, and forces mode 0700. A directory owned by
/// someone else is refused rather than reused, since the fallback may live in
/// a shared location such as `/tmp`.
fn ensure_private_dir(dir: &Path, uid: u32) -> io::Result<()> {
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let metadata = fs::symlink_metadata(dir)?;
    if !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "socket directory is not a directory",
        ));
    }
    if metadata.uid() != uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "socket directory is owned by another user",
        ));
    }
    if metadata.permissions().mode() & 0o777 != 0o700 {
        fs::set_permissions(dir, Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// A leftover socket from a crashed run is removed; a socket someone still
/// answers on is reported as in use; anything that is not a socket is left alone.
async fn remove_stale_socket(path: &Path) -> io::Result<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if !metadata.file_type().is_socket() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "endpoint path exists and is not a socket",
        ));
    }
    match UnixStream::connect(path).await {
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AddrInUse,
            "another process is serving the endpoint",
        )),
        Err(_) => fs::remove_file(path),
    }
}
