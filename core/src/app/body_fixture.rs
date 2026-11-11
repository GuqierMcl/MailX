//! Synthetic, local-only profile for core and host tests.
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
        Self::body_test_fixture_with_events(
            path,
            runtime,
            raws,
            Arc::new(|_: &crate::events::CoreEvent| {}),
        )
    }

    /// As [`Self::body_test_fixture`], delivering core events to `events`.
    pub fn body_test_fixture_with_events(
        path: &Path,
        runtime: Handle,
        raws: &[&[u8]],
        events: crate::events::Events,
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
        Self::body_fixture_open(path, runtime, raws, events, Arc::new(NoSecrets))
    }

    /// As [`Self::body_test_fixture`], with a secret store the test controls.
    #[cfg(test)]
    pub(crate) fn body_test_fixture_with_secrets(
        path: &Path,
        runtime: Handle,
        raws: &[&[u8]],
        secrets: Arc<dyn SecretStore>,
    ) -> Result<(Self, Vec<String>), AppError> {
        Self::body_fixture_open(
            path,
            runtime,
            raws,
            Arc::new(|_: &crate::events::CoreEvent| {}),
            secrets,
        )
    }

    /// Caches a synthetic translation of a fixture message: every text piece
    /// becomes `"[<target>] <text>"`, placeholders stay. Test-only; no network.
    pub fn body_test_translation(
        &self,
        email_id: &str,
        target: &str,
        include_quotes: bool,
    ) -> Result<(), AppError> {
        use crate::{
            settings::TranslationProviderId,
            storage::translations::NewTranslation,
            translate::{self, Piece, SEGMENTER_VERSION, wire},
        };
        let mut service = self.accounts()?;
        let identity = service.body_identity(email_id)?;
        let body = service
            .body_cache(&identity)?
            .ok_or_else(|| AppError::new(ErrorCode::BodyNotCached))?;
        let html = body
            .rendered
            .sanitized_html
            .ok_or_else(|| AppError::new(ErrorCode::BodyNotCached))?;
        let segments = translate::extract(&render::fold_quotes(&html, "x"), include_quotes)
            .map_err(|_| AppError::new(ErrorCode::Validation))?;
        let stored: Vec<Option<String>> = segments
            .iter()
            .map(|segment| {
                let pieces = segment
                    .pieces()
                    .iter()
                    .map(|piece| match piece {
                        Piece::Text(text) => Piece::Text(format!("[{target}] {text}")),
                        other => other.clone(),
                    })
                    .collect();
                Some(wire::encode(&translate::Segment::new(pieces)))
            })
            .collect();
        let written = service.translation_put(
            &identity,
            &(body.raw.clone(), body.sanitizer_version),
            &NewTranslation {
                message_id: email_id,
                target_language: target,
                include_quotes,
                raw_sha256: &body.raw.sha256,
                sanitizer_version: body.sanitizer_version,
                segmenter_version: SEGMENTER_VERSION,
                provider: TranslationProviderId::ALL[0],
                source_language: None,
                subject: None,
                segments: &stored,
                partial: false,
            },
        )?;
        if written {
            Ok(())
        } else {
            Err(AppError::new(ErrorCode::BodyNotCached))
        }
    }

    fn body_fixture_open(
        path: &Path,
        runtime: Handle,
        raws: &[&[u8]],
        events: crate::events::Events,
        secrets: Arc<dyn SecretStore>,
    ) -> Result<(Self, Vec<String>), AppError> {
        let core = Self::open(path, secrets, runtime, events)?;
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
