#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod body_protocol;
#[cfg(feature = "body-webview-test")]
mod body_webview_test;
mod commands;
#[cfg(not(feature = "body-webview-test"))]
mod secret_store;
mod state;
mod window_chrome;

#[cfg(not(feature = "body-webview-test"))]
use std::sync::Arc;
#[cfg(not(feature = "body-webview-test"))]
use tauri::{Emitter, EventTarget, Manager};

/// Pushed to the main window whenever an account's sync state changes.
#[cfg(not(feature = "body-webview-test"))]
const SYNC_STATE_EVENT: &str = "mailx://sync-state";

fn main() {
    #[cfg(feature = "body-webview-test")]
    body_webview_test::checkpoint("main entered");
    let builder = tauri::Builder::default()
        .append_invoke_initialization_script(include_str!("subframe_guard.js"))
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        );
    #[cfg(all(feature = "body-webview-test", target_os = "macos"))]
    let builder = builder.plugin(tauri_plugin_wdio_webdriver::init());
    body_protocol::register(builder)
        .setup(|app| {
            #[cfg(feature = "body-webview-test")]
            {
                crate::body_webview_test::setup(app)
            }
            #[cfg(not(feature = "body-webview-test"))]
            {
                let data_dir = app.path().app_data_dir().map_err(|_| {
                    mailx_core::AppError::new(mailx_core::ErrorCode::StorageUnavailable)
                });
                let secrets = Arc::new(secret_store::SystemSecretStore::new(
                    app.config().identifier.clone(),
                ));
                // Sync state changes only hint the UI to refetch; a lost event is harmless.
                let events = app.handle().clone();
                let sync_listener: mailx_core::SyncListener = Arc::new(move |sync_state| {
                    let _ = events.emit_to(
                        EventTarget::webview_window(window_chrome::MAIN_WINDOW),
                        SYNC_STATE_EVENT,
                        sync_state,
                    );
                });
                let state = state::DesktopState::new(data_dir, secrets, sync_listener);
                app.manage(state.clone());
                // Prepare the data directory before exposing business commands;
                // the task itself runs on Tauri's blocking pool.
                state.start_initialization();
                window_chrome::create_main_window(app.handle())?;
                Ok(())
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_get_startup_status,
            commands::app_retry_startup,
            commands::accounts_list,
            commands::accounts_get,
            commands::accounts_create,
            commands::accounts_update,
            commands::folders_list,
            commands::emails_list,
            commands::emails_get,
            commands::emails_fetch_body,
            commands::emails_prepare_body,
            commands::emails_body_links,
            commands::links_request,
            commands::links_open,
            commands::sync_states,
            commands::settings_get,
            commands::settings_update,
            commands::window_get_chrome,
            commands::window_perform,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run the MailX desktop host");
}
