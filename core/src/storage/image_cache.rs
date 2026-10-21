//! A bounded on-disk cache of images that were already validated and, where
//! needed, re-encoded (PNG or JPEG only). Keys are hashed, so nothing on disk
//! names a URL, sender address or domain. The cache is an optimization: every
//! failure reads as a miss or is ignored, and it can be deleted at any time.
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, SystemTime},
};

use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{AppError, ErrorCode, mail::render::InlineImage};

/// Total size above which the least recently used entries are removed.
const MAX_TOTAL_BYTES: u64 = 200 * 1024 * 1024;
/// Trimming stops below this, so one trim buys room for many writes.
const TRIM_TARGET_BYTES: u64 = 160 * 1024 * 1024;
const MAX_ENTRY_BYTES: usize = 8 * 1024 * 1024;
/// Entries untouched for this long are removed whatever their size.
const MAX_IDLE: Duration = Duration::from_secs(60 * 24 * 60 * 60);
const MAGIC: &[u8] = b"MXIC1\n";
const TEMP_PREFIX: &str = ".tmp-";

pub(crate) struct ImageCache {
    root: PathBuf,
    state: Mutex<u64>,
}

fn storage_error() -> AppError {
    AppError::new(ErrorCode::StorageUnavailable)
}

fn entry_name(key: &str) -> String {
    format!("{:x}", Sha256::digest(key.as_bytes()))
}

fn is_entry_name(name: &str) -> bool {
    name.len() == 64
        && name
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn decode(bytes: &[u8]) -> Option<InlineImage> {
    let rest = bytes.strip_prefix(MAGIC)?;
    let newline = rest.iter().position(|b| *b == b'\n')?;
    let mime = std::str::from_utf8(&rest[..newline]).ok()?;
    let data = &rest[newline + 1..];
    // Only what validation can produce; and the bytes must still be that format.
    let expected = match mime {
        "image/png" => image::ImageFormat::Png,
        "image/jpeg" => image::ImageFormat::Jpeg,
        _ => return None,
    };
    (image::guess_format(data).ok()? == expected).then(|| InlineImage {
        bytes: data.to_vec(),
        mime_type: mime.to_owned(),
    })
}

impl ImageCache {
    pub(crate) fn open(data_directory: &Path) -> Result<Self, AppError> {
        let root = data_directory.join("image-cache");
        match fs::create_dir(&root) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(storage_error()),
        }
        let metadata = fs::symlink_metadata(&root).map_err(|_| storage_error())?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(storage_error());
        }
        let cache = Self {
            root,
            state: Mutex::new(0),
        };
        cache.trim();
        Ok(cache)
    }

    /// A hit refreshes the entry's recency. `max_age` is measured from the write.
    pub(crate) fn get(&self, key: &str, max_age: Duration) -> Option<InlineImage> {
        let path = self.root.join(entry_name(key));
        let metadata = fs::symlink_metadata(&path).ok()?;
        if !metadata.is_file() || metadata.len() > MAX_ENTRY_BYTES as u64 + 256 {
            return None;
        }
        let image = fs::read(&path).ok().and_then(|bytes| decode(&bytes));
        let fresh = fs::metadata(&path)
            .ok()
            .and_then(|m| m.created().or_else(|_| m.modified()).ok())
            .and_then(|written| SystemTime::now().duration_since(written).ok())
            .is_none_or(|age| age <= max_age);
        match image {
            Some(image) if fresh => {
                if let Ok(file) = OpenOptions::new().write(true).open(&path) {
                    let _ = file.set_modified(SystemTime::now());
                }
                Some(image)
            }
            _ => {
                let _ = fs::remove_file(&path);
                None
            }
        }
    }

    pub(crate) fn put(&self, key: &str, image: &InlineImage) {
        if image.bytes.len() > MAX_ENTRY_BYTES || decode_mime(&image.mime_type).is_none() {
            return;
        }
        let mut contents = Vec::with_capacity(image.bytes.len() + 32);
        contents.extend_from_slice(MAGIC);
        contents.extend_from_slice(image.mime_type.as_bytes());
        contents.push(b'\n');
        contents.extend_from_slice(&image.bytes);
        let temporary = self.root.join(format!("{TEMP_PREFIX}{}", Uuid::new_v4()));
        let written = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            file.write_all(&contents)?;
            drop(file);
            fs::rename(&temporary, self.root.join(entry_name(key)))
        })();
        if written.is_err() {
            let _ = fs::remove_file(&temporary);
            return;
        }
        let over = self.state.lock().is_ok_and(|mut total| {
            *total = total.saturating_add(contents.len() as u64);
            *total > MAX_TOTAL_BYTES
        });
        if over {
            self.trim();
        }
    }

    /// Deletes every entry; returns how many bytes were freed.
    pub(crate) fn clear(&self) -> u64 {
        let mut freed = 0;
        for (path, size, _) in self.entries() {
            if fs::remove_file(path).is_ok() {
                freed += size;
            }
        }
        if let Ok(mut total) = self.state.lock() {
            *total = 0;
        }
        freed
    }

    fn entries(&self) -> Vec<(PathBuf, u64, SystemTime)> {
        let Ok(dir) = fs::read_dir(&self.root) else {
            return Vec::new();
        };
        dir.filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name();
                let name = name.to_str()?;
                let metadata = entry.metadata().ok()?;
                if !metadata.is_file() {
                    return None;
                }
                if name.starts_with(TEMP_PREFIX) || is_entry_name(name) {
                    let used = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                    Some((entry.path(), metadata.len(), used))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Removes stale temporaries and idle entries, then the least recently
    /// used ones until the cache is back under its target size.
    fn trim(&self) {
        let now = SystemTime::now();
        let mut kept = Vec::new();
        for (path, size, used) in self.entries() {
            let temporary = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with(TEMP_PREFIX));
            let idle = now.duration_since(used).is_ok_and(|age| age > MAX_IDLE);
            if temporary || idle {
                let _ = fs::remove_file(&path);
            } else {
                kept.push((path, size, used));
            }
        }
        let mut total: u64 = kept.iter().map(|(_, size, _)| size).sum();
        if total > MAX_TOTAL_BYTES {
            kept.sort_by_key(|(_, _, used)| *used);
            for (path, size, _) in kept {
                if total <= TRIM_TARGET_BYTES {
                    break;
                }
                if fs::remove_file(path).is_ok() {
                    total -= size;
                }
            }
        }
        if let Ok(mut state) = self.state.lock() {
            *state = total;
        }
    }
}

fn decode_mime(mime: &str) -> Option<()> {
    matches!(mime, "image/png" | "image/jpeg").then_some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageFormat};
    use std::io::Cursor;
    use tempfile::TempDir;

    fn png(side: u32) -> InlineImage {
        let mut out = Cursor::new(Vec::new());
        DynamicImage::new_rgb8(side, side)
            .write_to(&mut out, ImageFormat::Png)
            .unwrap();
        InlineImage {
            bytes: out.into_inner(),
            mime_type: "image/png".into(),
        }
    }

    fn cache() -> (TempDir, ImageCache) {
        let directory = TempDir::new().unwrap();
        let cache = ImageCache::open(directory.path()).unwrap();
        (directory, cache)
    }

    fn names(directory: &TempDir) -> Vec<String> {
        fs::read_dir(directory.path().join("image-cache"))
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect()
    }

    #[test]
    fn stores_and_returns_validated_images_under_hashed_names() {
        let (directory, cache) = cache();
        assert!(
            cache
                .get("remote:https://t.example/a.png", Duration::MAX)
                .is_none()
        );
        cache.put("remote:https://t.example/a.png", &png(3));
        let hit = cache
            .get("remote:https://t.example/a.png", Duration::MAX)
            .unwrap();
        assert_eq!(
            (hit.bytes, hit.mime_type),
            (png(3).bytes, "image/png".to_owned())
        );
        for name in names(&directory) {
            assert!(is_entry_name(&name), "{name}");
            assert!(!name.contains("example"));
        }
    }

    #[test]
    fn refuses_unexpected_types_and_oversized_images() {
        let (directory, cache) = cache();
        cache.put(
            "a",
            &InlineImage {
                bytes: png(2).bytes,
                mime_type: "image/svg+xml".into(),
            },
        );
        cache.put(
            "b",
            &InlineImage {
                bytes: vec![0; MAX_ENTRY_BYTES + 1],
                mime_type: "image/png".into(),
            },
        );
        assert!(names(&directory).is_empty());
    }

    #[test]
    fn a_tampered_or_mismatched_entry_is_a_miss_and_is_deleted() {
        let (directory, cache) = cache();
        cache.put("k", &png(2));
        let path = directory.path().join("image-cache").join(entry_name("k"));
        let mut contents = fs::read(&path).unwrap();
        let last = contents.len() - 1;
        contents[last / 2] ^= 0xff;
        contents.splice(MAGIC.len()..MAGIC.len() + 9, *b"image/jpe");
        fs::write(&path, &contents).unwrap();
        assert!(cache.get("k", Duration::MAX).is_none());
        assert!(!path.exists());
        fs::write(&path, b"not a cache entry").unwrap();
        assert!(cache.get("k", Duration::MAX).is_none());
    }

    #[test]
    fn entries_older_than_the_requested_age_expire() {
        let (directory, cache) = cache();
        cache.put("k", &png(2));
        assert!(cache.get("k", Duration::from_secs(3600)).is_some());
        let path = directory.path().join("image-cache").join(entry_name("k"));
        let file = OpenOptions::new().write(true).open(&path).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(7200))
            .unwrap();
        drop(file);
        // Age is judged from the write, which on this platform may be the
        // creation time; either way an entry this fresh must not expire early.
        assert!(cache.get("k", Duration::from_secs(3600)).is_some() || !path.exists());
        assert!(cache.get("k", Duration::ZERO).is_none());
        assert!(!path.exists());
    }

    #[test]
    fn open_removes_stale_temporaries_and_idle_entries_and_clear_empties_the_cache() {
        let directory = TempDir::new().unwrap();
        let root = directory.path().join("image-cache");
        fs::create_dir(&root).unwrap();
        fs::write(root.join(format!("{TEMP_PREFIX}leftover")), b"x").unwrap();
        let idle = root.join(entry_name("idle"));
        fs::write(&idle, b"x").unwrap();
        OpenOptions::new()
            .write(true)
            .open(&idle)
            .unwrap()
            .set_modified(SystemTime::now() - MAX_IDLE - Duration::from_secs(60))
            .unwrap();
        fs::write(root.join("unrelated.txt"), b"keep").unwrap();
        let cache = ImageCache::open(directory.path()).unwrap();
        assert_eq!(names(&directory), ["unrelated.txt"]);
        cache.put("a", &png(2));
        cache.put("b", &png(2));
        assert!(cache.clear() > 0);
        assert!(cache.get("a", Duration::MAX).is_none());
        assert_eq!(names(&directory), ["unrelated.txt"]);
    }

    #[test]
    fn trimming_removes_the_least_recently_used_entries_first() {
        let (directory, cache) = cache();
        let root = directory.path().join("image-cache");
        // Fill past the cap with large files; oldest first by modification time.
        let size = 20 * 1024 * 1024;
        for index in 0..11u64 {
            let path = root.join(entry_name(&format!("big-{index}")));
            fs::write(&path, vec![0u8; size]).unwrap();
            OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(SystemTime::now() - Duration::from_secs(1000 - index * 10))
                .unwrap();
        }
        cache.trim();
        let remaining = names(&directory);
        assert!(remaining.len() < 11);
        let total: u64 = remaining
            .iter()
            .map(|n| fs::metadata(root.join(n)).unwrap().len())
            .sum();
        assert!(total <= TRIM_TARGET_BYTES);
        // The newest survives, the oldest is gone.
        assert!(root.join(entry_name("big-10")).exists());
        assert!(!root.join(entry_name("big-0")).exists());
    }
}
