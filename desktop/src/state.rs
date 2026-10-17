use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

pub(crate) use mailx_core::StartupStatus;
use mailx_core::{AccountService, AppError, ErrorCode, secrets::SecretStore};

struct Inner {
    data_dir: Result<PathBuf, AppError>,
    secrets: Arc<dyn SecretStore>,
    service: Mutex<Option<AccountService>>,
    startup: Mutex<StartupStatus>,
    initialization_in_flight: AtomicBool,
}

#[derive(Clone)]
pub(crate) struct DesktopState(Arc<Inner>);

impl DesktopState {
    pub(crate) fn new(data_dir: Result<PathBuf, AppError>, secrets: Arc<dyn SecretStore>) -> Self {
        Self(Arc::new(Inner {
            data_dir,
            secrets,
            service: Mutex::new(None),
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
        F: FnOnce(&mut AccountService) -> Result<T, AppError> + Send + 'static,
    {
        if !self.is_ready() {
            return Err(AppError::new(ErrorCode::CoreNotReady));
        }
        let inner = self.0.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let mut service = inner
                .service
                .lock()
                .map_err(|_| AppError::new(ErrorCode::Internal))?;
            let service = service
                .as_mut()
                .ok_or_else(|| AppError::new(ErrorCode::CoreNotReady))?;
            operation(service)
        })
        .await
        .map_err(|_| AppError::new(ErrorCode::Internal))?
    }

    fn is_ready(&self) -> bool {
        matches!(
            self.0.startup.lock().ok().as_deref(),
            Some(StartupStatus::Ready { .. })
        )
    }

    fn open_service(&self) -> Result<AccountService, AppError> {
        let path = self.0.data_dir.as_ref().map_err(Clone::clone)?;
        AccountService::open(path, self.0.secrets.clone())
    }

    fn finish_initialization(&self, result: Result<AccountService, AppError>) {
        let status = match result {
            Ok(service) => match self.0.service.lock() {
                Ok(mut current) => {
                    *current = Some(service);
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
        );
        state.start_initialization();
        for _ in 0..100 {
            if matches!(state.startup_status(), StartupStatus::Ready { .. }) {
                let accounts = tauri::async_runtime::block_on(state.run(|service| service.list()))
                    .expect("ready service should accept business calls");
                assert!(accounts.is_empty());
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("startup did not reach Ready: {:?}", state.startup_status());
    }

    #[test]
    fn business_calls_are_rejected_before_ready() {
        let state = DesktopState::new(
            Err(AppError::new(ErrorCode::StorageUnavailable)),
            Arc::new(MemorySecrets::default()),
        );
        let error =
            tauri::async_runtime::block_on(state.run(|_| Ok::<_, AppError>(()))).unwrap_err();
        assert_eq!(error.code, ErrorCode::CoreNotReady);
    }

    #[test]
    fn retry_only_transitions_retryable_failed_state() {
        let state = DesktopState::new(
            Err(AppError::new(ErrorCode::StorageIncompatible)),
            Arc::new(MemorySecrets::default()),
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
