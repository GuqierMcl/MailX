//! Synthetic, local-only profile for core tests and the explicit WebView probe.
use std::{path::Path, sync::Arc};

use tokio::runtime::Handle;

use super::MailxCore;
use crate::{
    AppError, ErrorCode,
    mail::render,
    secrets::{SecretStore, SecretStoreError, SecretValue},
};

struct NoSecrets;

impl SecretStore for NoSecrets {
    fn get(&self, _key: &str) -> Result<Option<SecretValue>, SecretStoreError> {
        panic!("synthetic cached-body fixture must not access credentials")
    }
    fn set(&self, _key: &str, _value: &SecretValue) -> Result<(), SecretStoreError> {
        panic!("synthetic cached-body fixture must not write credentials")
    }
    fn delete(&self, _key: &str) -> Result<(), SecretStoreError> {
        panic!("synthetic cached-body fixture must not delete credentials")
    }
}

impl MailxCore {
    /// Build an isolated profile from synthetic MIME only. Existing databases
    /// are rejected before opening; no account exists when open starts workers.
    /// This API is absent from production builds and performs no network I/O.
    pub fn body_test_fixture(
        path: &Path,
        runtime: Handle,
        raws: &[&[u8]],
    ) -> Result<(Self, Vec<String>), AppError> {
        match std::fs::symlink_metadata(path) {
            Ok(metadata) => {
                if !metadata.is_dir()
                    || metadata.file_type().is_symlink()
                    || std::fs::read_dir(path)
                        .map_err(|_| AppError::new(ErrorCode::StorageUnavailable))?
                        .next()
                        .is_some()
                {
                    return Err(AppError::validation("testProfile"));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(AppError::new(ErrorCode::StorageUnavailable)),
        }
        let core = Self::open(path, Arc::new(NoSecrets), runtime, Arc::new(|_| {}))?;
        let identities = core
            .accounts()?
            .body_fixture_seed(&raws.iter().map(|raw| raw.len()).collect::<Vec<_>>())?;
        let blobs = core.accounts()?.body_blobs();
        for (identity, raw) in identities.iter().zip(raws) {
            // Filesystem and MIME work deliberately happen outside service.
            let rendered = render::render(raw, &identity.message_id)
                .map_err(|_| AppError::new(ErrorCode::Validation))?;
            let blob = blobs.put(raw)?;
            let mut service = core.accounts()?;
            if !service.body_begin(identity)? || !service.body_commit(identity, &blob, &rendered)? {
                return Err(AppError::new(ErrorCode::Internal));
            }
        }
        Ok((
            core,
            identities
                .into_iter()
                .map(|identity| identity.message_id)
                .collect(),
        ))
    }
}
