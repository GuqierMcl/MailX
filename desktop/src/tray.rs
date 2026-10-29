//! Native recovery entries; Linux support depends on the current desktop host.

use std::sync::Mutex;

use mailx_core::SupportedLocale;
use serde::Deserialize;
use tauri::{
    AppHandle, Emitter, EventTarget, Manager,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

use crate::{
    lifecycle,
    state::DesktopState,
    window_chrome::{self, MAIN_WINDOW},
};

const ID: &str = "mailx-tray";

#[derive(Default)]
pub(crate) struct TrayState {
    menu: Mutex<Option<TrayMenu>>,
    #[cfg(target_os = "linux")]
    host_available: std::sync::atomic::AtomicBool,
    #[cfg(target_os = "linux")]
    session_locked: std::sync::atomic::AtomicBool,
}

struct TrayMenu {
    open: MenuItem<tauri::Wry>,
    compose: MenuItem<tauri::Wry>,
    sync: MenuItem<tauri::Wry>,
    settings: MenuItem<tauri::Wry>,
    quit: MenuItem<tauri::Wry>,
}

#[derive(Deserialize)]
struct Labels {
    open: String,
    compose: String,
    sync: String,
    settings: String,
    quit: String,
}

fn labels(locale: SupportedLocale) -> Labels {
    serde_json::from_str(match locale {
        SupportedLocale::ZhCn => include_str!("../../locales/zh-CN/desktop.json"),
        SupportedLocale::En => include_str!("../../locales/en/desktop.json"),
    })
    .expect("embedded desktop menu resources")
}

pub(crate) fn setup(app: &AppHandle) {
    refresh(app);
    #[cfg(target_os = "linux")]
    {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                let status =
                    tauri::async_runtime::spawn_blocking(crate::tray_support::desktop_status)
                        .await
                        .unwrap_or_default();
                let state = app.state::<TrayState>();
                let host_changed = state
                    .host_available
                    .swap(status.host_available, std::sync::atomic::Ordering::SeqCst)
                    != status.host_available;
                let lock_changed = state
                    .session_locked
                    .swap(status.session_locked, std::sync::atomic::Ordering::SeqCst)
                    != status.session_locked;
                if host_changed || lock_changed {
                    refresh(&app);
                }
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        });
    }
}

/// Schedule GTK/AppKit operations on the UI thread. Menu items are updated in
/// place: AppIndicator does not support replacing a tray's menu.
pub(crate) fn refresh(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || refresh_on_main(&handle));
}

fn refresh_on_main(app: &AppHandle) {
    let Some(state) = app.try_state::<TrayState>() else {
        return;
    };
    let (locale, show_menu_bar, desktop_ready) = lifecycle::tray_options(app);
    let quitting = crate::compose::is_quitting(app);
    let labels = labels(locale);
    let wanted = !cfg!(target_os = "macos") || show_menu_bar;
    let mut slot = state.menu.lock().unwrap_or_else(|error| error.into_inner());
    if wanted && slot.is_none() {
        *slot = create(app, &labels).ok();
    }
    let created = slot.is_some();
    if let Some(menu) = slot.as_ref() {
        for (item, text) in [
            (&menu.open, &labels.open),
            (&menu.compose, &labels.compose),
            (&menu.sync, &labels.sync),
            (&menu.settings, &labels.settings),
            (&menu.quit, &labels.quit),
        ] {
            let _ = item.set_text(text);
        }
        let _ = menu.compose.set_enabled(desktop_ready && !quitting);
        let _ = menu.settings.set_enabled(desktop_ready && !quitting);
        let _ = menu.sync.set_enabled(desktop_ready && !quitting);
        let _ = menu.quit.set_enabled(!quitting);
    }
    if let Some(icon) = app.tray_by_id(ID) {
        let _ = icon.set_visible(wanted);
    }
    drop(slot);
    #[cfg(target_os = "linux")]
    let available = created
        && state
            .host_available
            .load(std::sync::atomic::Ordering::SeqCst);
    #[cfg(target_os = "windows")]
    let available = created;
    #[cfg(target_os = "macos")]
    let available = {
        let _ = created;
        true
    }; // Dock is always retained.
    lifecycle::set_background_available(app, available);
}

fn create(app: &AppHandle, labels: &Labels) -> tauri::Result<TrayMenu> {
    let item = |id, label: &str| MenuItem::with_id(app, id, label, true, None::<&str>);
    let items = TrayMenu {
        open: item("tray-open", &labels.open)?,
        compose: item("tray-compose", &labels.compose)?,
        sync: item("tray-sync", &labels.sync)?,
        settings: item("tray-settings", &labels.settings)?,
        quit: item("tray-quit", &labels.quit)?,
    };
    let separator = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(
        app,
        &[
            &items.open,
            &items.compose,
            &items.sync,
            &items.settings,
            &separator,
            &items.quit,
        ],
    )?;
    #[cfg(target_os = "macos")]
    let image =
        tauri::image::Image::from_bytes(include_bytes!("../icons/tray/trayTemplate@2x.png"))?;
    // The bounded app tile stays legible on both light and dark notification
    // areas; the menu bar uses the OS-tinted template instead.
    #[cfg(not(target_os = "macos"))]
    let image = tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png"))?;
    TrayIconBuilder::with_id(ID)
        .icon(image)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("MailX")
        .menu(&menu)
        .show_menu_on_left_click(!cfg!(target_os = "windows"))
        .on_menu_event(|app, event| match event.id.as_ref() {
            "tray-open" => window_chrome::focus_main_window(app),
            "tray-quit" => crate::compose::request_quit(app),
            "tray-settings" => desktop_action(app, "settings"),
            "tray-compose" => desktop_action(app, "compose"),
            "tray-sync" => refresh_mail(app),
            _ => {}
        })
        .on_tray_icon_event(|icon, event| {
            if cfg!(target_os = "windows")
                && matches!(
                    event,
                    TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    }
                )
            {
                window_chrome::focus_main_window(icon.app_handle());
            }
        })
        .build(app)?;
    Ok(items)
}

fn desktop_action(app: &AppHandle, action: &str) {
    if !lifecycle::tray_options(app).2 || crate::compose::is_quitting(app) {
        return;
    }
    window_chrome::focus_main_window(app);
    let _ = app.emit_to(
        EventTarget::webview_window(MAIN_WINDOW),
        "mailx://desktop-action",
        action,
    );
}

fn refresh_mail(app: &AppHandle) {
    if crate::compose::is_quitting(app) {
        return;
    }
    let state = app.state::<DesktopState>().inner().clone();
    tauri::async_runtime::spawn(async move {
        let _ = state
            .run(|core| {
                for account in core.list_accounts()? {
                    core.refresh_sync(&account.id)?;
                }
                Ok(())
            })
            .await;
    });
}

/// Lock knowledge only defers automatic recovery; explicit Open still works.
pub(crate) fn session_locked(app: &AppHandle) -> bool {
    #[cfg(target_os = "linux")]
    {
        app.state::<TrayState>()
            .session_locked
            .load(std::sync::atomic::Ordering::SeqCst)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = app;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn both_native_menus_have_all_recovery_and_quit_actions() {
        for locale in [SupportedLocale::ZhCn, SupportedLocale::En] {
            let labels = labels(locale);
            for text in [
                labels.open,
                labels.compose,
                labels.sync,
                labels.settings,
                labels.quit,
            ] {
                assert!(!text.trim().is_empty());
            }
        }
    }
}
