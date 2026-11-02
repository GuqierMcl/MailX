//! The discovery file the bridge reads to find the endpoint and the token.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const DISCOVERY_FILE_NAME: &str = "mcp-discovery.json";
pub const DISCOVERY_VERSION: u32 = 1;

/// Contents of `mcp-discovery.json`. Field names are part of the bridge
/// contract (docs/contracts/mcp.md).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Discovery {
    pub version: u32,
    /// Absolute socket path (Unix) or pipe name (Windows).
    pub endpoint: String,
    pub token: String,
    pub pid: u32,
    /// Unique per start; lets a stopping server remove only its own file.
    pub instance_id: String,
}

pub fn discovery_path(data_dir: &Path) -> PathBuf {
    data_dir.join(DISCOVERY_FILE_NAME)
}

/// Writes through a temporary file in the same directory and renames, so a
/// reader never sees a partial file. On Unix the file is 0600; on Windows it
/// inherits the data directory's ACL, which is already user-only.
pub fn write_atomic(path: &Path, discovery: &Discovery) -> io::Result<()> {
    let tmp = path.with_extension(format!("tmp-{}", discovery.instance_id));
    let bytes = serde_json::to_vec(discovery).map_err(io::Error::other)?;
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        // create_new: never reuse a leftover file whose mode we did not set.
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

pub fn read(path: &Path) -> io::Result<Discovery> {
    let bytes = fs::read(path)?;
    serde_json::from_slice(&bytes).map_err(io::Error::other)
}

/// Removes the file only when it still belongs to `instance_id`, so an old
/// server shutting down cannot delete the file a newer one just wrote.
pub fn remove_if_mine(path: &Path, instance_id: &str) -> bool {
    match read(path) {
        Ok(found) if found.instance_id == instance_id => fs::remove_file(path).is_ok(),
        _ => false,
    }
}
