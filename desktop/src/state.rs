use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

pub(crate) use mailx_core::StartupStatus;
use mailx_core::{
    Activity, AiService, AppError, ErrorCode, Events, MailxCore, SettingsStore,
    secrets::SecretStore,
};

struct Inner {
    data_dir: Result<PathBuf, AppError>,
    secrets: Arc<dyn SecretStore>,
    events: Events,
    core: Mutex<Option<MailxCore>>,
    /// The latest word on how the window is being used. Window events can come
    /// before the core has opened, so it is kept here and handed over when it does.
    activity: Mutex<Activity>,
    settings: Mutex<Option<Arc<SettingsStore>>>,
    /// AI provider settings need only the settings, secrets and data
    /// directory, so they work even when the mail store could not open.
    ai: Mutex<Option<Arc<AiService>>>,
    startup: Mutex<StartupStatus>,
    initialization_in_flight: AtomicBool,
    /// Called each time the core becomes ready (also after a retry).
    ready_hook: Mutex<Option<ReadyHook>>,
}

type ReadyHook = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone)]
pub(crate) struct DesktopState(Arc<Inner>);

impl DesktopState {
    pub(crate) fn new(
        data_dir: Result<PathBuf, AppError>,
        secrets: Arc<dyn SecretStore>,
        events: Events,
    ) -> Self {
        Self(Arc::new(Inner {
            data_dir,
            secrets,
            events,
            core: Mutex::new(None),
            activity: Mutex::new(Activity::default()),
            settings: Mutex::new(None),
            ai: Mutex::new(None),
            // The setup hook immediately schedules this first attempt. Keep
            // the state initializing during the short scheduling window.
            startup: Mutex::new(StartupStatus::Initializing { revision: 1 }),
            initialization_in_flight: AtomicBool::new(false),
            ready_hook: Mutex::new(None),
        }))
    }

    /// Registers work to run whenever the core becomes ready, for services
    /// that need the core (the local MCP server). Replaces an earlier hook.
    pub(crate) fn set_ready_hook(&self, hook: impl Fn() + Send + Sync + 'static) {
        if let Ok(mut slot) = self.0.ready_hook.lock() {
            *slot = Some(Arc::new(hook));
        }
    }

    pub(crate) fn data_dir(&self) -> Result<PathBuf, AppError> {
        self.0.data_dir.clone()
    }

    /// The persisted `mcp.enabled` switch; false when settings cannot be read.
    pub(crate) fn mcp_enabled(&self) -> bool {
        self.settings_store()
            .and_then(|store| store.get())
            .is_ok_and(|snapshot| snapshot.settings.mcp.enabled)
    }

    /// Called from the login-startup blocking worker, never on the UI thread.
    pub(crate) fn silent_start(&self) -> bool {
        self.settings_store()
            .and_then(|store| store.get())
            .is_ok_and(|snapshot| snapshot.settings.general.silent_start)
    }

    /// Account the shared tools use when a call names none.
    pub(crate) fn default_account_id(&self) -> String {
        self.settings_store()
            .and_then(|store| store.get())
            .map(|snapshot| snapshot.settings.compose.default_account_id)
            .unwrap_or_default()
    }

    /// The sending tools use the same undo window as the composer.
    pub(crate) fn undo_send_seconds(&self) -> Result<u32, AppError> {
        Ok(self
            .settings_store()?
            .get()?
            .settings
            .compose
            .undo_send_seconds)
    }

    /// Remembers how the window is being used and tells the core if it is open.
    /// Repeats cost nothing: the core only reacts to a real change.
    ///
    /// Lock order is core, then activity, here and in `finish_initialization`.
    /// Both are held together so a core that opens right now gets this value
    /// and a value set right now reaches that core; neither can be lost.
    pub(crate) fn set_activity(&self, activity: Activity) {
        let Ok(core) = self.0.core.lock() else {
            return;
        };
        let Ok(mut current) = self.0.activity.lock() else {
            return;
        };
        *current = activity;
        if let Some(core) = core.as_ref() {
            core.set_activity(activity);
        }
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

    /// The current notification preferences, for code already off the async
    /// runtime. `None` when settings cannot be read: then nothing is shown.
    pub(crate) fn notification_settings(
        &self,
    ) -> Option<(
        mailx_core::NotificationSettings,
        mailx_core::SupportedLocale,
    )> {
        let snapshot = self.notification_snapshot()?;
        Some((snapshot.settings.notifications, snapshot.effective_locale))
    }

    pub(crate) fn notification_snapshot(&self) -> Option<mailx_core::SettingsSnapshot> {
        self.settings_store().ok()?.get().ok()
    }

    /// The AI provider service, created on first use.
    pub(crate) fn ai(&self) -> Result<Arc<AiService>, AppError> {
        let store = self.settings_store()?;
        let mut slot = self
            .0
            .ai
            .lock()
            .map_err(|_| AppError::new(ErrorCode::Internal))?;
        if let Some(service) = slot.as_ref() {
            return Ok(service.clone());
        }
        let directory = self.0.data_dir.as_ref().map_err(Clone::clone)?;
        let service = Arc::new(AiService::new(store, self.0.secrets.clone(), directory));
        *slot = Some(service.clone());
        Ok(service)
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
        let store = Arc::new(SettingsStore::open_with_locale(
            directory,
            crate::locale::system_locale(),
        )?);
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

    fn is_ready(&self) -> bool {
        matches!(
            self.0.startup.lock().ok().as_deref(),
            Some(StartupStatus::Ready { .. })
        )
    }

    fn open_service(&self) -> Result<MailxCore, AppError> {
        let path = self.0.data_dir.as_ref().map_err(Clone::clone)?;
        let tauri::async_runtime::RuntimeHandle::Tokio(runtime) = tauri::async_runtime::handle();
        MailxCore::open(path, self.0.secrets.clone(), runtime, self.0.events.clone())
    }

    fn finish_initialization(&self, result: Result<MailxCore, AppError>) {
        let status = match result {
            Ok(core) => match self.0.core.lock() {
                Ok(mut current) => {
                    // Apply what the window said while the core was still opening.
                    if let Ok(activity) = self.0.activity.lock() {
                        core.set_activity(*activity);
                    }
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
        let ready = matches!(status, StartupStatus::Ready { .. });
        if let Ok(mut current) = self.0.startup.lock() {
            *current = status;
        }
        if ready {
            let hook = self.0.ready_hook.lock().ok().and_then(|slot| slot.clone());
            if let Some(hook) = hook {
                hook();
            }
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
            Arc::new(|_: &mailx_core::CoreEvent| {}),
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
            Arc::new(|_: &mailx_core::CoreEvent| {}),
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

    fn open_and_wait_until_ready(state: &DesktopState) {
        state.start_initialization();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !matches!(state.startup_status(), StartupStatus::Ready { .. }) {
            assert!(
                !matches!(state.startup_status(), StartupStatus::Failed { .. }),
                "startup failed"
            );
            assert!(std::time::Instant::now() < deadline, "never became ready");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn the_windows_activity_reaches_the_core_whether_it_is_told_before_or_after_startup() {
        // Told while the core is still closed: it is applied the moment the core opens.
        let early_directory = tempfile::tempdir().unwrap();
        let early = DesktopState::new(
            Ok(early_directory.path().to_owned()),
            Arc::new(MemorySecrets::default()),
            Arc::new(|_: &mailx_core::CoreEvent| {}),
        );
        early.set_activity(Activity::Background);
        open_and_wait_until_ready(&early);
        assert_eq!(early.core().unwrap().activity(), Activity::Background);

        // Told after startup: it goes straight through, and repeats change nothing.
        let late_directory = tempfile::tempdir().unwrap();
        let late = DesktopState::new(
            Ok(late_directory.path().to_owned()),
            Arc::new(MemorySecrets::default()),
            Arc::new(|_: &mailx_core::CoreEvent| {}),
        );
        open_and_wait_until_ready(&late);
        assert_eq!(late.core().unwrap().activity(), Activity::Active);
        for _ in 0..3 {
            late.set_activity(Activity::Background);
        }
        assert_eq!(late.core().unwrap().activity(), Activity::Background);
        late.set_activity(Activity::Active);
        assert_eq!(late.core().unwrap().activity(), Activity::Active);
    }

    #[test]
    fn business_calls_are_rejected_before_ready() {
        let state = DesktopState::new(
            Err(AppError::new(ErrorCode::StorageUnavailable)),
            Arc::new(MemorySecrets::default()),
            Arc::new(|_: &mailx_core::CoreEvent| {}),
        );
        // Async connection diagnostics use this path directly, before any probe.
        assert_eq!(state.core().err().unwrap().code, ErrorCode::CoreNotReady);
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
            Arc::new(|_: &mailx_core::CoreEvent| {}),
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
            Arc::new(|_: &mailx_core::CoreEvent| {}),
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
            Arc::new(|_: &mailx_core::CoreEvent| {}),
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
