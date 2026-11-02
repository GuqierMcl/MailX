//! Where the local endpoint lives.
//!
//! Unix domain socket paths are limited to roughly 104 bytes (macOS) or 108
//! (Linux), and an application data directory can easily exceed that. When the
//! preferred path (`<data>/mcp/mcp.sock`) is too long the socket moves to a
//! per-user runtime directory; the discovery file always records the final
//! absolute path, so the bridge never needs to know which one was chosen.

use std::path::{Path, PathBuf};

/// Dedicated subdirectory of the data directory that holds the preferred
/// socket, so the data directory's own permissions are never touched.
pub const SOCKET_DIR_NAME: &str = "mcp";
/// Preferred socket file name inside [`SOCKET_DIR_NAME`].
pub const SOCKET_FILE_NAME: &str = "mcp.sock";

/// Conservative limit in bytes; below both `sun_path` sizes with room for NUL.
pub const MAX_SOCKET_PATH_BYTES: usize = 100;

/// Where a fallback socket directory would be rooted for this user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FallbackRoot {
    /// Per-user directory that will be created with mode 0700.
    pub dir: PathBuf,
}

/// Preferred path (`<data>/mcp/mcp.sock`, length checked in full) when it fits, otherwise `fallback.dir/mcp-<hash>.sock`.
///
/// The hash (of the data directory) keeps two data directories, for example a
/// development override next to the real one, from fighting over one socket.
pub fn choose_socket_path(
    data_dir: &Path,
    fallback: &FallbackRoot,
    max_bytes: usize,
) -> (PathBuf, bool) {
    let preferred = data_dir.join(SOCKET_DIR_NAME).join(SOCKET_FILE_NAME);
    if path_len(&preferred) <= max_bytes {
        return (preferred, false);
    }
    let name = format!("mcp-{:08x}.sock", short_hash(data_dir));
    (fallback.dir.join(name), true)
}

fn path_len(path: &Path) -> usize {
    path.as_os_str().len()
}

/// FNV-1a over the path bytes. Stable across runs and Rust versions, unlike
/// `DefaultHasher`, and only used to disambiguate file names.
fn short_hash(path: &Path) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in path.as_os_str().as_encoded_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

/// Per-user fallback directory from the environment.
///
/// macOS: `$TMPDIR/mailx-<uid>` (the per-user, per-boot temporary directory,
/// short enough for sockets). Linux: `$XDG_RUNTIME_DIR/mailx`, which is already
/// private to the user. Without either variable: `/tmp/mailx-<uid>`.
#[cfg(unix)]
pub fn fallback_root(uid: u32) -> FallbackRoot {
    let env_dir = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    let dir = if cfg!(target_os = "macos") {
        env_dir("TMPDIR")
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(format!("mailx-{uid}"))
    } else if let Some(runtime) = env_dir("XDG_RUNTIME_DIR") {
        runtime.join("mailx")
    } else {
        PathBuf::from("/tmp").join(format!("mailx-{uid}"))
    };
    FallbackRoot { dir }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fallback() -> FallbackRoot {
        FallbackRoot {
            dir: PathBuf::from("/run/user/1000/mailx"),
        }
    }

    #[test]
    fn short_data_dir_keeps_the_preferred_path() {
        let (path, moved) = choose_socket_path(Path::new("/data/mailx"), &fallback(), 100);
        assert_eq!(path, PathBuf::from("/data/mailx/mcp/mcp.sock"));
        assert!(!moved);
    }

    #[test]
    fn path_exactly_at_the_limit_is_kept() {
        let dir = PathBuf::from("/d");
        let preferred_len = dir
            .join(SOCKET_DIR_NAME)
            .join(SOCKET_FILE_NAME)
            .as_os_str()
            .len();
        let (_, moved) = choose_socket_path(&dir, &fallback(), preferred_len);
        assert!(!moved);
        let (_, moved) = choose_socket_path(&dir, &fallback(), preferred_len - 1);
        assert!(moved);
    }

    #[test]
    fn long_data_dir_moves_to_the_fallback_directory() {
        let long = PathBuf::from(format!("/{}", "a".repeat(120)));
        let (path, moved) = choose_socket_path(&long, &fallback(), 100);
        assert!(moved);
        assert_eq!(path.parent().unwrap(), Path::new("/run/user/1000/mailx"));
        assert!(
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("mcp-")
        );
        assert!(path.as_os_str().len() <= 100);
    }

    #[test]
    fn fallback_names_differ_per_data_dir_and_are_stable() {
        let a = PathBuf::from(format!("/{}/one", "a".repeat(120)));
        let b = PathBuf::from(format!("/{}/two", "a".repeat(120)));
        let (pa, _) = choose_socket_path(&a, &fallback(), 100);
        let (pb, _) = choose_socket_path(&b, &fallback(), 100);
        assert_ne!(pa, pb);
        assert_eq!(pa, choose_socket_path(&a, &fallback(), 100).0);
    }
}
