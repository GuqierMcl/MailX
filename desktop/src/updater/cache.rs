//! Completed update packages and the receipt for a quit-time installation.
//!
//! Cached metadata never supplies an installer or a trusted public key. A caller
//! must first obtain a fresh `Update`, and its verifier checks every reused byte.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tauri_plugin_updater::Update;
use uuid::Uuid;

use super::policy::{ReleaseDetails, cap_notes, is_newer};

const METADATA_LIMIT: u64 = 128 * 1024;
const PACKAGE_LIMIT: u64 = 1024 * 1024 * 1024;
const PACKAGE_METADATA: &str = "package.json";
const INSTALL_RECEIPT: &str = "install-attempt.json";

#[derive(Clone)]
pub(crate) struct Cache {
    directory: PathBuf,
    operations: Arc<Mutex<()>>,
}

#[derive(Debug)]
pub(crate) struct CachedPackage {
    pub(crate) bytes: Vec<u8>,
    /// Only a user-started download carries consent across an app restart.
    pub(crate) manual: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(crate) struct InstallFailure {
    pub(crate) version: String,
    pub(crate) details: ReleaseDetails,
}

#[derive(Deserialize, Serialize)]
struct PackageMetadata {
    schema: u32,
    version: String,
    target: String,
    os: String,
    arch: String,
    url: String,
    signature: String,
    file: String,
    manual: bool,
}

impl PackageMetadata {
    fn matches(&self, update: &Update) -> bool {
        self.schema == 1
            && self.version == update.version
            && self.target == update.target
            && self.os == std::env::consts::OS
            && self.arch == std::env::consts::ARCH
            && self.url == update.download_url.as_str()
            && self.signature == update.signature
            && package_name(&self.file)
    }
}

impl Cache {
    pub(crate) fn new(data_dir: PathBuf) -> Self {
        Self {
            directory: data_dir.join("updater"),
            operations: Arc::new(Mutex::new(())),
        }
    }

    fn lock(&self) -> MutexGuard<'_, ()> {
        self.operations
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    /// The metadata pointer is committed last, so a partial download/write can
    /// never replace the previously committed package. Clones share a lock so
    /// pruning cannot remove a receipt's temporary file during normal quit.
    pub(crate) fn store(&self, update: &Update, bytes: &[u8], manual: bool) -> io::Result<()> {
        let _guard = self.lock();
        if bytes.len() as u64 > PACKAGE_LIMIT {
            return Err(invalid_data());
        }
        update.verify(bytes).map_err(|_| invalid_data())?;
        fs::create_dir_all(&self.directory)?;
        let file = format!("package-{}.bin", Uuid::new_v4());
        let package = self.directory.join(&file);
        atomic_write(&package, bytes)?;
        let metadata = PackageMetadata {
            schema: 1,
            version: update.version.clone(),
            target: update.target.clone(),
            os: std::env::consts::OS.into(),
            arch: std::env::consts::ARCH.into(),
            url: update.download_url.as_str().into(),
            signature: update.signature.clone(),
            file: file.clone(),
            manual,
        };
        if let Err(error) = write_json(&self.directory.join(PACKAGE_METADATA), &metadata) {
            let _ = fs::remove_file(package);
            return Err(error);
        }
        self.clean_packages(Some(&file))
    }

    /// A mismatched, truncated or corrupted cache is a cache miss, never a
    /// usable update. Re-verification uses the current built-in key and policy.
    pub(crate) fn load(&self, update: &Update) -> io::Result<Option<CachedPackage>> {
        let _guard = self.lock();
        let metadata = match read_json::<PackageMetadata>(&self.directory.join(PACKAGE_METADATA)) {
            Ok(Some(metadata)) if metadata.matches(update) => metadata,
            Ok(None) => return Ok(None),
            Ok(Some(_)) | Err(_) => {
                self.clear_package_unlocked()?;
                return Ok(None);
            }
        };
        let path = self.directory.join(&metadata.file);
        let bytes = match read_limited(&path, PACKAGE_LIMIT) {
            Ok(Some(bytes)) if update.verify(&bytes).is_ok() => bytes,
            _ => {
                self.clear_package_unlocked()?;
                return Ok(None);
            }
        };
        Ok(Some(CachedPackage {
            bytes,
            manual: metadata.manual,
        }))
    }

    /// Write before calling the platform installer: on Windows it can terminate
    /// this process without returning. The next version acknowledges success.
    pub(crate) fn mark_install_attempt(
        &self,
        version: &str,
        details: &ReleaseDetails,
    ) -> io::Result<()> {
        let _guard = self.lock();
        fs::create_dir_all(&self.directory)?;
        let mut details = details.clone();
        details.notes = cap_notes(details.notes);
        write_json(
            &self.directory.join(INSTALL_RECEIPT),
            &InstallFailure {
                version: version.into(),
                details,
            },
        )
    }

    /// An attempted installation is only a failure while the running version
    /// is older. This also handles a user repairing the app with a manual install.
    pub(crate) fn failure(&self, current_version: &str) -> io::Result<Option<InstallFailure>> {
        let _guard = self.lock();
        let failure = match read_json::<InstallFailure>(&self.directory.join(INSTALL_RECEIPT)) {
            Ok(Some(failure)) => failure,
            Ok(None) => return Ok(None),
            Err(_) => {
                self.clear_failure_unlocked()?;
                return Ok(None);
            }
        };
        if !is_newer(current_version, &failure.version) {
            self.clear_unlocked()?;
            return Ok(None);
        }
        let mut failure = failure;
        failure.details.current = current_version.into();
        failure.details.notes = cap_notes(failure.details.notes);
        Ok(Some(failure))
    }

    pub(crate) fn clear_failure(&self) -> io::Result<()> {
        let _guard = self.lock();
        self.clear_failure_unlocked()
    }

    fn clear_failure_unlocked(&self) -> io::Result<()> {
        remove_if_present(&self.directory.join(INSTALL_RECEIPT))
    }

    #[cfg(test)]
    fn clear_package(&self) -> io::Result<()> {
        let _guard = self.lock();
        self.clear_package_unlocked()
    }

    fn clear_package_unlocked(&self) -> io::Result<()> {
        remove_if_present(&self.directory.join(PACKAGE_METADATA))?;
        self.clean_packages(None)
    }

    pub(crate) fn clear(&self) -> io::Result<()> {
        let _guard = self.lock();
        self.clear_unlocked()
    }

    fn clear_unlocked(&self) -> io::Result<()> {
        self.clear_failure_unlocked()?;
        self.clear_package_unlocked()
    }

    fn clean_packages(&self, keep: Option<&str>) -> io::Result<()> {
        let entries = match fs::read_dir(&self.directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if Some(name) != keep && (package_name(name) || temporary_name(name)) {
                remove_if_present(&entry.path())?;
            }
        }
        Ok(())
    }
}

fn package_name(name: &str) -> bool {
    name.strip_prefix("package-")
        .and_then(|name| name.strip_suffix(".bin"))
        .is_some_and(|id| Uuid::parse_str(id).is_ok())
}

fn temporary_name(name: &str) -> bool {
    name.strip_prefix("write-")
        .and_then(|name| name.strip_suffix(".tmp"))
        .is_some_and(|id| Uuid::parse_str(id).is_ok())
}

fn invalid_data() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid updater cache")
}

fn remove_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn read_limited(path: &Path, limit: u64) -> io::Result<Option<Vec<u8>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if !metadata.is_file() || metadata.len() > limit {
        return Err(invalid_data());
    }
    let mut bytes = Vec::new();
    File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(invalid_data());
    }
    Ok(Some(bytes))
}

fn read_json<T: DeserializeOwned>(path: &Path) -> io::Result<Option<T>> {
    read_limited(path, METADATA_LIMIT)?
        .map(|bytes| serde_json::from_slice(&bytes).map_err(|_| invalid_data()))
        .transpose()
}

fn write_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|_| invalid_data())?;
    if bytes.len() as u64 > METADATA_LIMIT {
        return Err(invalid_data());
    }
    atomic_write(path, &bytes)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let directory = path.parent().ok_or_else(invalid_data)?;
    let temporary = directory.join(format!("write-{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        #[cfg(unix)]
        File::open(directory)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tauri_plugin_updater::UpdaterExt;

    // Public throwaway test vector from tauri-plugin-updater's offline tests:
    // payload `test`, signed with a trusted version comment for 2.0.0.
    const PUBLIC_KEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEE0OUVGNDc0NkZBNkU2RDcKUldUWDVxWnZkUFNlcERHV2cyYno4bjZBclFuQk13Z1FLMWdlVm9zb3JjYkx6Y3VuRCtLSitjWWU=";
    const SIGNATURE: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVUWDVxWnZkUFNlcFBGMnpzanZQL0wvVFF0QkxSbldSdmFxKzRvbUsxVUV1Z3RRcC9VU3JHaUkxdjc2MnNEeTNpZDNhb3I3YlJDOUVUeWVvdDZsTUhIcldvWjVoalVNMVFjPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkxNjQ5MDUxCWZpbGU6cGF5bG9hZAl2ZXJzaW9uOjIuMC4wCkREcTJDTE9FbDVjUW1DMmVKUkhsblpQVlZpT1pzdTFDanBQS1FvZWk0QS9YQmZrRHFHTzBCK1FNcmNySGdFUTVxVjNtbmU3NXRkRldoSnBxVVhoOUFBPT0=";

    fn update() -> Update {
        let mut context = tauri::test::mock_context(tauri::test::noop_assets());
        context.config_mut().plugins.0.insert(
            "updater".into(),
            json!({ "pubkey": PUBLIC_KEY, "endpoints": ["https://example.invalid/update"] }),
        );
        let app = tauri::test::mock_builder()
            .plugin(tauri_plugin_updater::Builder::new().build())
            .build(context)
            .unwrap();
        app.updater_builder()
            .target("cache-test")
            .executable_path("runtime-path/app")
            .version_comparator(|_, _| true)
            .no_proxy()
            .configure_client(|_| panic!("cache tests must never use the network"))
            .build()
            .unwrap()
            .restore_update(json!({
                "version": "2.0.0",
                "notes": "Release notes",
                "platforms": {
                    "cache-test": {
                        "url": "https://example.invalid/package",
                        "signature": SIGNATURE
                    }
                }
            }))
            .unwrap()
            .unwrap()
    }

    fn details() -> ReleaseDetails {
        ReleaseDetails {
            current: "1.0.0".into(),
            notes: Some("Release notes".into()),
            date: None,
        }
    }

    #[test]
    fn completed_packages_survive_reopening_and_keep_their_origin() {
        let directory = tempfile::tempdir().unwrap();
        let cache = Cache::new(directory.path().into());
        let update = update();
        assert!(cache.load(&update).unwrap().is_none());
        cache.store(&update, b"test", true).unwrap();
        let reopened = Cache::new(directory.path().into());
        let loaded = reopened.load(&update).unwrap().unwrap();
        assert_eq!(loaded.bytes, b"test");
        assert!(loaded.manual);

        cache.store(&update, b"test", false).unwrap();
        assert!(!cache.load(&update).unwrap().unwrap().manual);
        let packages = fs::read_dir(&cache.directory)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| package_name(&entry.file_name().to_string_lossy()))
            .count();
        assert_eq!(packages, 1);
    }

    #[test]
    fn invalid_download_cannot_replace_a_committed_package() {
        let directory = tempfile::tempdir().unwrap();
        let cache = Cache::new(directory.path().into());
        let update = update();
        cache.store(&update, b"test", true).unwrap();
        assert!(cache.store(&update, b"tampered", false).is_err());
        let loaded = cache.load(&update).unwrap().unwrap();
        assert_eq!(loaded.bytes, b"test");
        assert!(loaded.manual);
    }

    #[test]
    fn corrupted_bytes_are_verified_again_and_discarded() {
        let directory = tempfile::tempdir().unwrap();
        let cache = Cache::new(directory.path().into());
        let update = update();
        cache.store(&update, b"test", false).unwrap();
        let metadata: PackageMetadata = read_json(&cache.directory.join(PACKAGE_METADATA))
            .unwrap()
            .unwrap();
        fs::write(cache.directory.join(metadata.file), b"evil").unwrap();
        assert!(cache.load(&update).unwrap().is_none());
        assert!(!cache.directory.join(PACKAGE_METADATA).exists());
    }

    #[test]
    fn changed_release_identity_is_a_cache_miss() {
        let directory = tempfile::tempdir().unwrap();
        let cache = Cache::new(directory.path().into());
        let original = update();
        for change in ["version", "target", "url", "signature", "arch", "file"] {
            cache.store(&original, b"test", false).unwrap();
            let path = cache.directory.join(PACKAGE_METADATA);
            let mut metadata: serde_json::Value = read_json(&path).unwrap().unwrap();
            metadata[change] = json!("../unexpected");
            write_json(&path, &metadata).unwrap();
            assert!(cache.load(&original).unwrap().is_none(), "{change}");
        }
    }

    #[test]
    fn partial_writes_do_not_replace_the_metadata_pointer() {
        let directory = tempfile::tempdir().unwrap();
        let cache = Cache::new(directory.path().into());
        let update = update();
        cache.store(&update, b"test", false).unwrap();
        let orphan = cache
            .directory
            .join(format!("package-{}.bin", Uuid::new_v4()));
        let partial = cache
            .directory
            .join(format!("write-{}.tmp", Uuid::new_v4()));
        fs::write(&orphan, b"partially committed").unwrap();
        fs::write(&partial, b"partial write").unwrap();
        assert_eq!(cache.load(&update).unwrap().unwrap().bytes, b"test");
        cache.store(&update, b"test", false).unwrap();
        assert!(!orphan.exists());
        assert!(!partial.exists());
    }

    #[test]
    fn failed_installation_is_reported_until_the_running_version_catches_up() {
        let directory = tempfile::tempdir().unwrap();
        let cache = Cache::new(directory.path().into());
        cache.store(&update(), b"test", false).unwrap();
        cache.mark_install_attempt("2.0.0", &details()).unwrap();
        let failure = cache.failure("1.5.0").unwrap().unwrap();
        assert_eq!(failure.version, "2.0.0");
        assert_eq!(failure.details.current, "1.5.0");
        assert_eq!(failure.details.notes, details().notes);
        assert!(cache.failure("2.0.0").unwrap().is_none());
        assert!(!cache.directory.join(INSTALL_RECEIPT).exists());
        assert!(!cache.directory.join(PACKAGE_METADATA).exists());
    }

    #[test]
    fn package_eviction_and_failure_acknowledgement_are_independent() {
        let directory = tempfile::tempdir().unwrap();
        let cache = Cache::new(directory.path().into());
        let update = update();
        cache.store(&update, b"test", false).unwrap();
        cache.mark_install_attempt("2.0.0", &details()).unwrap();
        cache.clear_package().unwrap();
        assert!(cache.failure("1.0.0").unwrap().is_some());
        cache.store(&update, b"test", false).unwrap();
        cache.clear_failure().unwrap();
        assert!(cache.failure("1.0.0").unwrap().is_none());
        assert!(cache.load(&update).unwrap().is_some());
    }

    #[test]
    fn malformed_records_are_removed_without_blocking_updates() {
        let directory = tempfile::tempdir().unwrap();
        let cache = Cache::new(directory.path().into());
        fs::create_dir_all(&cache.directory).unwrap();
        fs::write(cache.directory.join(PACKAGE_METADATA), b"{").unwrap();
        assert!(cache.load(&update()).unwrap().is_none());
        fs::write(cache.directory.join(INSTALL_RECEIPT), b"{").unwrap();
        assert!(cache.failure("1.0.0").unwrap().is_none());
    }

    #[test]
    fn concurrent_package_cleanup_does_not_interrupt_install_receipts() {
        let directory = tempfile::tempdir().unwrap();
        let cache = Cache::new(directory.path().into());
        let writer = cache.clone();
        let receipt = std::thread::spawn(move || {
            for _ in 0..20 {
                writer.mark_install_attempt("2.0.0", &details()).unwrap();
            }
        });
        for _ in 0..20 {
            cache.clear_package().unwrap();
        }
        receipt.join().unwrap();
        assert!(cache.failure("1.0.0").unwrap().is_some());
    }
}
