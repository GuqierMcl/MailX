//! The unread number on the app icon: the Dock on macOS, the launcher on
//! Linux (through the Unity launcher API, where the desktop supports it).
//! Windows has no badge count; there the module does nothing.
//!
//! The number is the core's (`MailxCore::badge_count`), recomputed in the
//! background after events that can change unread counts, so it stays right
//! while the main window is hidden. Bursts are coalesced and the system is
//! only called when the number changes.

/// Whether this platform can show a number on the app icon.
pub(crate) const SUPPORTED: bool = cfg!(any(target_os = "macos", target_os = "linux"));

/// Starts the background updater; also run once the core is ready.
pub(crate) fn setup(app: &tauri::AppHandle) {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    imp::setup(app);
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let _ = app;
}

/// Asks for the number to be recomputed soon. Cheap; call it after anything
/// that may change unread counts or the badge settings.
pub(crate) fn refresh(app: &tauri::AppHandle) {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    imp::refresh(app);
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    let _ = app;
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod imp {
    use std::sync::Arc;
    use std::time::Duration;

    use tauri::{AppHandle, Manager};
    use tokio::sync::Notify;

    use crate::state::DesktopState;
    #[cfg(target_os = "linux")]
    use crate::window_chrome::MAIN_WINDOW;

    /// How long a burst of changes (a sync round, a batch of marks) settles
    /// before the number is read.
    const SETTLE: Duration = Duration::from_millis(300);

    struct BadgeState {
        wake: Arc<Notify>,
    }

    pub(super) fn setup(app: &AppHandle) {
        let wake = Arc::new(Notify::new());
        app.manage(BadgeState { wake: wake.clone() });
        let compute_app = app.clone();
        let apply_app = app.clone();
        tauri::async_runtime::spawn(super::run(
            wake,
            SETTLE,
            move || {
                let app = compute_app.clone();
                async move {
                    tauri::async_runtime::spawn_blocking(move || current_count(&app))
                        .await
                        .ok()
                        .flatten()
                }
            },
            move |count| {
                let app = apply_app.clone();
                async move { apply_badge(&app, count).await }
            },
        ));
        let ready_app = app.clone();
        app.state::<DesktopState>()
            .add_ready_hook(move || refresh(&ready_app));
        refresh(app);
    }

    /// Updates the macOS Dock tile directly without requiring a live main
    /// window, so unread counts stay accurate while the UI is released.
    #[cfg(target_os = "macos")]
    async fn apply_badge(app: &AppHandle, count: Option<i64>) -> bool {
        use objc2_app_kit::NSApplication;
        use objc2_foundation::{MainThreadMarker, NSString};

        let (tx, rx) = tokio::sync::oneshot::channel();
        let label = count.map(|value| value.to_string());
        if app
            .run_on_main_thread(move || {
                let applied = MainThreadMarker::new().is_some_and(|mtm| {
                    let tile = NSApplication::sharedApplication(mtm).dockTile();
                    let value = label.as_deref().map(NSString::from_str);
                    tile.setBadgeLabel(value.as_deref());
                    true
                });
                let _ = tx.send(applied);
            })
            .is_err()
        {
            return false;
        }
        rx.await.unwrap_or(false)
    }

    #[cfg(target_os = "linux")]
    async fn apply_badge(app: &AppHandle, count: Option<i64>) -> bool {
        app.get_webview_window(MAIN_WINDOW)
            .is_some_and(|window| window.set_badge_count(count).is_ok())
    }

    pub(super) fn refresh(app: &AppHandle) {
        if let Some(state) = app.try_state::<BadgeState>() {
            state.wake.notify_one();
        }
    }

    /// `None` while the core is not open or settings cannot be read: the
    /// icon then keeps what it shows.
    fn current_count(app: &AppHandle) -> Option<u32> {
        let state = app.try_state::<DesktopState>()?;
        let settings = state.badge_settings()?;
        if !settings.enabled {
            return Some(0);
        }
        state.core().ok()?.badge_count(&settings).ok()
    }
}

/// What the icon shows, so an unchanged number costs no system call.
#[cfg(any(test, target_os = "macos", target_os = "linux"))]
#[derive(Default)]
struct Shown(Option<u32>);

#[cfg(any(test, target_os = "macos", target_os = "linux"))]
impl Shown {
    /// The badge to set for `count` (`None` clears it), or nothing when the
    /// icon already shows that number.
    fn change(&self, count: u32) -> Option<Option<i64>> {
        (self.0 != Some(count)).then(|| (count > 0).then(|| i64::from(count)))
    }
}

/// Waits for a wake-up, lets the burst settle for `settle`, reads the number
/// and applies it if it changed. A wake-up during the work is kept and runs
/// the loop once more, so no change is missed.
#[cfg(any(test, target_os = "macos", target_os = "linux"))]
async fn run<C, F, A, AF>(
    wake: std::sync::Arc<tokio::sync::Notify>,
    settle: std::time::Duration,
    mut compute: C,
    mut apply: A,
) where
    C: FnMut() -> F,
    F: std::future::Future<Output = Option<u32>>,
    A: FnMut(Option<i64>) -> AF,
    AF: std::future::Future<Output = bool>,
{
    let mut shown = Shown::default();
    loop {
        wake.notified().await;
        tokio::time::sleep(settle).await;
        let Some(count) = compute().await else {
            continue;
        };
        if let Some(badge) = shown.change(count)
            && apply(badge).await
        {
            shown = Shown(Some(count));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use tokio::sync::Notify;

    use super::*;

    #[test]
    fn zero_clears_and_only_changes_reach_the_system() {
        let mut shown = Shown::default();
        // The first number always goes out, so a stale badge is cleared too.
        assert_eq!(shown.change(0), Some(None));
        shown = Shown(Some(0));
        assert_eq!(shown.change(0), None);
        assert_eq!(shown.change(12), Some(Some(12)));
        shown = Shown(Some(12));
        assert_eq!(shown.change(12), None);
        assert_eq!(shown.change(0), Some(None));
        assert_eq!(shown.change(u32::MAX), Some(Some(i64::from(u32::MAX))));
    }

    #[tokio::test]
    async fn a_burst_is_read_once_and_an_unchanged_number_is_not_applied_again() {
        let wake = Arc::new(Notify::new());
        let reads = Arc::new(Mutex::new(0u32));
        let number = Arc::new(Mutex::new(Some(3u32)));
        let applied = Arc::new(Mutex::new(Vec::new()));
        let task = tokio::spawn(run(
            wake.clone(),
            Duration::from_millis(40),
            {
                let reads = reads.clone();
                let number = number.clone();
                move || {
                    *reads.lock().unwrap() += 1;
                    let value = *number.lock().unwrap();
                    async move { value }
                }
            },
            {
                let applied = applied.clone();
                move |badge| {
                    applied.lock().unwrap().push(badge);
                    async { true }
                }
            },
        ));
        let settle = || tokio::time::sleep(Duration::from_millis(200));

        for _ in 0..5 {
            wake.notify_one();
        }
        settle().await;
        assert_eq!(*reads.lock().unwrap(), 1);
        assert_eq!(*applied.lock().unwrap(), [Some(3)]);

        // Same number: read again, nothing sent to the system.
        wake.notify_one();
        settle().await;
        assert_eq!(*reads.lock().unwrap(), 2);
        assert_eq!(applied.lock().unwrap().len(), 1);

        // Core not ready: nothing changes, and the next number still goes out.
        *number.lock().unwrap() = None;
        wake.notify_one();
        settle().await;
        *number.lock().unwrap() = Some(0);
        wake.notify_one();
        settle().await;
        assert_eq!(*applied.lock().unwrap(), [Some(3), None]);
        task.abort();
    }

    #[tokio::test]
    async fn a_failed_update_is_tried_again_next_time() {
        let wake = Arc::new(Notify::new());
        let attempts = Arc::new(Mutex::new(0u32));
        let task = tokio::spawn(run(
            wake.clone(),
            Duration::from_millis(10),
            || async { Some(5) },
            {
                let attempts = attempts.clone();
                move |_| {
                    let mut attempts = attempts.lock().unwrap();
                    *attempts += 1;
                    let ok = *attempts > 1;
                    async move { ok }
                }
            },
        ));
        for _ in 0..3 {
            wake.notify_one();
            tokio::time::sleep(Duration::from_millis(80)).await;
        }
        // Failed, succeeded, then unchanged.
        assert_eq!(*attempts.lock().unwrap(), 2);
        task.abort();
    }
}
