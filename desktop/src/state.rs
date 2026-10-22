use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

pub(crate) use mailx_core::StartupStatus;
use mailx_core::{
    AppError, ErrorCode, FlagListener, MailxCore, SettingsStore, SyncListener, secrets::SecretStore,
};

struct Inner {
    data_dir: Result<PathBuf, AppError>,
    secrets: Arc<dyn SecretStore>,
    sync_listener: SyncListener,
    flag_listener: FlagListener,
    core: Mutex<Option<MailxCore>>,
    settings: Mutex<Option<Arc<SettingsStore>>>,
    startup: Mutex<StartupStatus>,
    initialization_in_flight: AtomicBool,
}

#[derive(Clone)]
pub(crate) struct DesktopState(Arc<Inner>);

impl DesktopState {
    pub(crate) fn new(
        data_dir: Result<PathBuf, AppError>,
        secrets: Arc<dyn SecretStore>,
        sync_listener: SyncListener,
        flag_listener: FlagListener,
    ) -> Self {
        Self(Arc::new(Inner {
            data_dir,
            secrets,
            sync_listener,
            flag_listener,
            core: Mutex::new(None),
            settings: Mutex::new(None),
            // The setup hook immediately schedules this first attempt. Keep
            // the state initializing during the short scheduling window.
            startup: Mutex::new(StartupStatus::Initializing { revision: 1 }),
            initialization_in_flight: AtomicBool::new(false),
        }))
    }

    pub(crate) fn startup_status(&self) -> StartupStatus {
        self.0
            .startup
            .lock()
            .map(|status| status.clone())
            .unwrap_or_else(|_| StartupStatus::Failed {
                revision: 0,
                error: AppError::new(ErrorCode::Internal),
                can_retry: false,
            })
    }

    /// Start the initial preparation attempt. The same method is used by the
    /// retry command; the status transition and atomic in-flight bit make
    /// concurrent retry clicks single-flight.
    pub(crate) fn start_initialization(&self) {
        if self
            .0
            .initialization_in_flight
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        let state = self.clone();
        tauri::async_runtime::spawn(async move {
            let result = tauri::async_runtime::spawn_blocking({
                let state = state.clone();
                move || state.open_service()
            })
            .await;
            let result = match result {
                Ok(result) => result,
                // A panic in the blocking worker must become a terminal,
                // inspectable startup failure rather than a stuck spinner.
                Err(_) => Err(AppError::new(ErrorCode::Internal)),
            };
            state.finish_initialization(result);
        });
    }

    /// Move a retryable Failed snapshot back to Initializing and schedule one
    /// attempt. Non-retryable failures are intentionally left untouched.
    pub(crate) fn retry_startup(&self) -> StartupStatus {
        let should_start = match self.0.startup.lock() {
            Ok(mut status) => match &*status {
                StartupStatus::Failed {
                    can_retry: true, ..
                } => {
                    let revision = revision(&status).saturating_add(1);
                    *status = StartupStatus::Initializing { revision };
                    true
                }
                _ => false,
            },
            Err(_) => false,
        };
        if should_start {
            self.start_initialization();
        }
        self.startup_status()
    }

    /// Run a business operation only after startup has published a complete
    /// service. This does not wait for initialization: callers receive a
    /// fixed not-ready error and the UI can use the status command to explain
    /// the condition.
    pub(crate) async fn run<T, F>(&self, operation: F) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce(&MailxCore) -> Result<T, AppError> + Send + 'static,
    {
        let core = self.core()?;
        // The core serializes store access itself; this only keeps the
        // blocking call off the async runtime.
        tauri::async_runtime::spawn_blocking(move || operation(&core))
            .await
            .map_err(|_| AppError::new(ErrorCode::Internal))?
    }

    /// Preferences only need the data directory, so they stay available when
    /// the business service could not start. A failed open is retried on the
    /// next call.
    pub(crate) async fn settings<T, F>(&self, operation: F) -> Result<T, AppError>
    where
        T: Send + 'static,
        F: FnOnce(&SettingsStore) -> Result<T, AppError> + Send + 'static,
    {
        let state = self.clone();
        tauri::async_runtime::spawn_blocking(move || operation(&*state.settings_store()?))
            .await
            .map_err(|_| AppError::new(ErrorCode::Internal))?
    }

    fn settings_store(&self) -> Result<Arc<SettingsStore>, AppError> {
        let mut slot = self
            .0
            .settings
            .lock()
            .map_err(|_| AppError::new(ErrorCode::Internal))?;
        if let Some(store) = slot.as_ref() {
            return Ok(store.clone());
        }
        let directory = self.0.data_dir.as_ref().map_err(Clone::clone)?;
        let store = Arc::new(SettingsStore::open(directory)?);
        *slot = Some(store.clone());
        Ok(store)
    }

    /// Clone a ready core for its asynchronous body operations. No database or
    /// credential access takes place on the WebView event thread here.
    pub(crate) fn core(&self) -> Result<MailxCore, AppError> {
        if !self.is_ready() {
            return Err(AppError::new(ErrorCode::CoreNotReady));
        }
        self.0
            .core
            .lock()
            .map_err(|_| AppError::new(ErrorCode::Internal))?
            .clone()
            .ok_or_else(|| AppError::new(ErrorCode::CoreNotReady))
    }

    #[cfg(feature = "body-webview-test")]
    pub(crate) fn for_body_test(core: MailxCore, directory: PathBuf) -> Self {
        struct NoSecrets;
        impl SecretStore for NoSecrets {
            fn get(
                &self,
                _: &str,
            ) -> Result<
                Option<mailx_core::secrets::SecretValue>,
                mailx_core::secrets::SecretStoreError,
            > {
                panic!("test must not read credentials")
            }
            fn set(
                &self,
                _: &str,
                _: &mailx_core::secrets::SecretValue,
            ) -> Result<(), mailx_core::secrets::SecretStoreError> {
                panic!("test must not write credentials")
            }
            fn delete(&self, _: &str) -> Result<(), mailx_core::secrets::SecretStoreError> {
                panic!("test must not delete credentials")
            }
        }
        let state = Self::new(
            Ok(directory),
            Arc::new(NoSecrets),
            Arc::new(|_| {}),
            Arc::new(|_| {}),
            Arc::new(|_| {}),
        );
        state.finish_initialization(Ok(core));
        state
    }

    fn is_ready(&self) -> bool {
        matches!(
            self.0.startup.lock().ok().as_deref(),
            Some(StartupStatus::Ready { .. })
        )
    }

    fn open_service(&self) -> Result<MailxCore, AppError> {
        let path = self.0.data_dir.as_ref().map_err(Clone::clone)?;
        let tauri::async_runtime::RuntimeHandle::Tokio(runtime) = tauri::async_runtime::handle();
        MailxCore::open(
            path,
            self.0.secrets.clone(),
            runtime,
            self.0.sync_listener.clone(),
            self.0.flag_listener.clone(),
        )
    }

    fn finish_initialization(&self, result: Result<MailxCore, AppError>) {
        let status = match result {
            Ok(core) => match self.0.core.lock() {
                Ok(mut current) => {
                    *current = Some(core);
                    StartupStatus::Ready {
                        revision: self.next_revision(),
                    }
                }
                Err(_) => StartupStatus::Failed {
                    revision: self.next_revision(),
                    error: AppError::new(ErrorCode::Internal),
                    can_retry: false,
                },
            },
            Err(error) => StartupStatus::Failed {
                revision: self.next_revision(),
                can_retry: can_retry(&error),
                error,
            },
        };
        // Mark the worker finished before publishing Failed. A concurrent
        // retry can then only observe the old Initializing snapshot and will
        // not strand a new attempt between the status store and this flag.
        self.0
            .initialization_in_flight
            .store(false, Ordering::Release);
        if let Ok(mut current) = self.0.startup.lock() {
            *current = status;
        }
    }

    fn next_revision(&self) -> u32 {
        self.0
            .startup
            .lock()
            .map(|status| revision(&status).saturating_add(1))
            .unwrap_or(1)
    }
}

fn revision(status: &StartupStatus) -> u32 {
    match status {
        StartupStatus::Initializing { revision }
        | StartupStatus::Ready { revision }
        | StartupStatus::Failed { revision, .. } => *revision,
    }
}

fn can_retry(error: &AppError) -> bool {
    matches!(
        error.code,
        ErrorCode::StorageUnavailable
            | ErrorCode::AlreadyOpen
            | ErrorCode::SecretStoreLocked
            | ErrorCode::SecretStoreUnavailable
            | ErrorCode::Internal
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use mailx_core::secrets::{SecretStoreError, SecretValue};
    use std::collections::HashMap;

    #[derive(Default)]
    struct MemorySecrets(Mutex<HashMap<String, String>>);

    impl SecretStore for MemorySecrets {
        fn get(&self, key: &str) -> Result<Option<SecretValue>, SecretStoreError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(key)
                .cloned()
                .map(SecretValue::new))
        }
        fn set(&self, key: &str, value: &SecretValue) -> Result<(), SecretStoreError> {
            self.0
                .lock()
                .unwrap()
                .insert(key.to_owned(), value.expose_secret().to_owned());
            Ok(())
        }
        fn delete(&self, key: &str) -> Result<(), SecretStoreError> {
            self.0.lock().unwrap().remove(key);
            Ok(())
        }
    }

    #[test]
    fn failed_data_directory_is_retryable_and_does_not_open_business_service() {
        let state = DesktopState::new(
            Err(AppError::new(ErrorCode::StorageUnavailable)),
            Arc::new(MemorySecrets::default()),
            Arc::new(|_| {}),
            Arc::new(|_| {}),
        );
        assert!(matches!(
            state.startup_status(),
            StartupStatus::Initializing { .. }
        ));
        state.start_initialization();
        // The task is asynchronous; this assertion only verifies that the
        // scheduling call cannot synchronously publish a fake Ready state.
        assert!(!matches!(
            state.startup_status(),
            StartupStatus::Ready { .. }
        ));
    }

    #[test]
    fn startup_publishes_ready_before_business_call() {
        let directory = tempfile::tempdir().unwrap();
        let state = DesktopState::new(
            Ok(directory.path().to_owned()),
            Arc::new(MemorySecrets::default()),
            Arc::new(|_| {}),
            Arc::new(|_| {}),
        );
        state.start_initialization();
        // This checks readiness ordering, not startup performance. Migrations
        // and durable writes can exceed 500 ms on a busy CI filesystem.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            match state.startup_status() {
                StartupStatus::Ready { .. } => break,
                failure @ StartupStatus::Failed { .. } => {
                    panic!("startup failed: {failure:?}");
                }
                status @ StartupStatus::Initializing { .. } => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "startup did not reach Ready: {status:?}"
                    );
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let accounts = tauri::async_runtime::block_on(state.run(|core| core.list_accounts()))
            .expect("ready service should accept business calls");
        assert!(accounts.is_empty());
    }

    #[test]
    fn business_calls_are_rejected_before_ready() {
        let state = DesktopState::new(
            Err(AppError::new(ErrorCode::StorageUnavailable)),
            Arc::new(MemorySecrets::default()),
            Arc::new(|_| {}),
            Arc::new(|_| {}),
        );
        let error =
            tauri::async_runtime::block_on(state.run(|_| Ok::<_, AppError>(()))).unwrap_err();
        assert_eq!(error.code, ErrorCode::CoreNotReady);
    }

    #[test]
    fn settings_work_before_the_business_service_is_ready() {
        let directory = tempfile::tempdir().unwrap();
        let state = DesktopState::new(
            Ok(directory.path().to_owned()),
            Arc::new(MemorySecrets::default()),
            Arc::new(|_| {}),
            Arc::new(|_| {}),
        );
        let mut settings = mailx_core::AppSettings::default();
        settings.appearance.interface_font = "Inter, PingFang SC".into();
        let saved =
            tauri::async_runtime::block_on(state.settings(move |store| store.update(settings)))
                .unwrap();
        assert!(saved.stored);
        assert!(!state.is_ready());
        let loaded = tauri::async_runtime::block_on(state.settings(|store| store.get())).unwrap();
        assert_eq!(loaded, saved);
    }

    #[test]
    fn settings_report_an_unavailable_data_directory() {
        let state = DesktopState::new(
            Err(AppError::new(ErrorCode::StorageUnavailable)),
            Arc::new(MemorySecrets::default()),
            Arc::new(|_| {}),
            Arc::new(|_| {}),
        );
        let error =
            tauri::async_runtime::block_on(state.settings(|store| store.get())).unwrap_err();
        assert_eq!(error.code, ErrorCode::StorageUnavailable);
    }

    #[test]
    fn retry_only_transitions_retryable_failed_state() {
        let state = DesktopState::new(
            Err(AppError::new(ErrorCode::StorageIncompatible)),
            Arc::new(MemorySecrets::default()),
            Arc::new(|_| {}),
            Arc::new(|_| {}),
        );
        *state.0.startup.lock().unwrap() = StartupStatus::Failed {
            revision: 4,
            error: AppError::new(ErrorCode::StorageIncompatible),
            can_retry: false,
        };
        assert_eq!(state.retry_startup(), state.startup_status());
        assert!(matches!(
            state.startup_status(),
            StartupStatus::Failed { .. }
        ));
    }
}
