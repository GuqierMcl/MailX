#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod attachment_save;
mod body_protocol;
#[cfg(feature = "body-webview-test")]
mod body_webview_test;
mod commands;
#[cfg(not(feature = "body-webview-test"))]
mod notifications;
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
/// Pushed after a read / starred write-back round, for refreshing markers and
/// telling the user a star stayed on this device.
#[cfg(not(feature = "body-webview-test"))]
const FLAG_EVENT: &str = "mailx://flags";

fn main() {
    #[cfg(feature = "body-webview-test")]
    body_webview_test::checkpoint("main entered");
    let builder = tauri::Builder::default();
    // Registered first, as the plugin requires. A second launch hands over to the
    // running instance and exits, so it never reaches the data-directory lock.
    #[cfg(not(feature = "body-webview-test"))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
        window_chrome::focus_main_window(app)
    }));
    #[cfg(not(feature = "body-webview-test"))]
    let builder = builder.plugin(tauri_plugin_notification::init());
    let builder = builder
        .plugin(tauri_plugin_dialog::init())
        .append_invoke_initialization_script(include_str!("subframe_guard.js"))
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        );
    #[cfg(not(feature = "body-webview-test"))]
    let builder = builder.plugin(window_chrome::window_state_plugin());
    // Packaged builds only: development keeps the inspector and reload.
    #[cfg(all(windows, not(feature = "body-webview-test")))]
    let builder = if cfg!(debug_assertions) {
        builder
    } else {
        builder.plugin(window_chrome::browser_defaults_plugin())
    };
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
                // Sync and write-back events only hint the UI to refetch or
                // tell the user something; a lost one is harmless.
                let handle = app.handle().clone();
                let settings_handle = app.handle().clone();
                let events: mailx_core::Events =
                    Arc::new(move |event: &mailx_core::CoreEvent| match event {
                        mailx_core::CoreEvent::Sync(sync_state) => {
                            let _ = handle.emit_to(
                                EventTarget::webview_window(window_chrome::MAIN_WINDOW),
                                SYNC_STATE_EVENT,
                                sync_state,
                            );
                        }
                        mailx_core::CoreEvent::FlagWriteBack(written) => {
                            let _ = handle.emit_to(
                                EventTarget::webview_window(window_chrome::MAIN_WINDOW),
                                FLAG_EVENT,
                                written,
                            );
                        }
                        // TODO(sending step 4): forward as `mailx://outbox` and
                        // `mailx://drafts` once the composer and outbox views exist.
                        mailx_core::CoreEvent::Outbox(_)
                        | mailx_core::CoreEvent::DraftChanged(_) => {}
                        mailx_core::CoreEvent::NewMail(mail) => {
                            let app = settings_handle.clone();
                            notifications::new_mail(&handle, mail.clone(), move || {
                                app.try_state::<state::DesktopState>()?
                                    .notification_settings()
                            });
                        }
                    });
                let state = state::DesktopState::new(data_dir, secrets, events);
                app.manage(state.clone());
                // Prepare the data directory before exposing business commands;
                // the task itself runs on Tauri's blocking pool.
                state.start_initialization();
                notifications::on_click(app.handle());
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
            commands::accounts_update_connection,
            commands::accounts_update_profile,
            commands::folders_list,
            commands::emails_list,
            commands::emails_get,
            commands::emails_fetch_body,
            commands::emails_prepare_body,
            commands::emails_release_body,
            commands::emails_body_links,
            commands::links_request,
            commands::links_open,
            commands::sync_states,
            commands::sync_refresh,
            commands::emails_set_flag,
            commands::attachments_save,
            commands::avatars_get,
            commands::image_cache_clear,
            commands::settings_get,
            commands::settings_update,
            commands::notifications_test,
            commands::notifications_preview_sound,
            commands::window_get_chrome,
            commands::window_perform,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run the MailX desktop host");
}
