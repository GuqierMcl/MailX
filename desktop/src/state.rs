use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
};

use mailx_core::{AccountService, AppError, ErrorCode, secrets::SecretStore};

#[derive(Clone)]
pub(crate) struct DesktopState(Arc<Inner>);

struct Inner {
    data_dir: Result<PathBuf, AppError>,
    secrets: Arc<dyn SecretStore>,
    service: Mutex<Option<AccountService>>,
}

impl DesktopState {
    pub(crate) fn new(data_dir: Result<PathBuf, AppError>, secrets: Arc<dyn SecretStore>) -> Self {
        Self(Arc::new(Inner {
            data_dir,
            secrets,
            service: Mutex::new(None),
        }))
    }

    /// Open/recover and all SQLite/keychain calls run off the webview event loop.
    /// Failed opens can be retried after the user unlocks the credential store.
    pub(crate) async fn run<T, F>(&self, operation: F) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce(&mut AccountService) -> Result<T, AppError> + Send + 'static,
    {
        let inner = self.0.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let mut service = inner
                .service
                .lock()
                .map_err(|_| AppError::new(ErrorCode::Internal))?;
            if service.is_none() {
                let path = inner.data_dir.as_ref().map_err(Clone::clone)?;
                *service = Some(AccountService::open(path, inner.secrets.clone())?);
            }
            operation(
                service
                    .as_mut()
                    .ok_or_else(|| AppError::new(ErrorCode::Internal))?,
            )
        })
        .await
        .map_err(|_| AppError::new(ErrorCode::Internal))?
    }
}
