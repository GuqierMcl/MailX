#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod attachment_save;
mod body_protocol;
#[cfg(feature = "body-webview-test")]
mod body_webview_test;
mod commands;
mod compose;
mod compose_files;
#[cfg_attr(feature = "body-webview-test", allow(dead_code))]
mod lifecycle;
mod locale;
#[cfg(not(feature = "body-webview-test"))]
mod notifications;
#[cfg(not(feature = "body-webview-test"))]
mod secret_store;
#[cfg(not(feature = "body-webview-test"))]
mod secret_vault;
mod state;
#[cfg(not(feature = "body-webview-test"))]
mod storage_location;
#[cfg_attr(feature = "body-webview-test", allow(dead_code))]
mod tray;
#[cfg(target_os = "linux")]
mod tray_support;
#[cfg_attr(feature = "body-webview-test", allow(dead_code))]
mod updater;
mod window_access;
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

/// List previews fetched for rows the main window asked about.
#[cfg(not(feature = "body-webview-test"))]
const PREVIEWS_EVENT: &str = "mailx://email-previews";

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
    #[cfg(not(feature = "body-webview-test"))]
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
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
                let storage = app
                    .path()
                    .app_data_dir()
                    .map_err(|_| {
                        mailx_core::AppError::new(mailx_core::ErrorCode::StorageUnavailable)
                    })
                    .and_then(|dir| {
                        storage_location::storage(
                            &dir,
                            std::env::var_os(storage_location::DATA_DIR_OVERRIDE).as_deref(),
                        )
                    });
                let secret_service = storage
                    .as_ref()
                    .map(|storage| storage.secret_service.clone())
                    .unwrap_or_else(|_| storage_location::SECRET_SERVICE.to_owned());
                let data_dir = storage.map(|storage| storage.data_dir);
                let secrets = secret_store::system_store(secret_service);
                // Sync and write-back events only hint the UI to refetch or
                // tell the user something; a lost one is harmless.
                let handle = app.handle().clone();
                let settings_handle = app.handle().clone();
                let notification_accounts =
                    Arc::new(notifications::AccountNotifications::default());
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
                        mailx_core::CoreEvent::Previews(previews) => {
                            let _ = handle.emit_to(
                                EventTarget::webview_window(window_chrome::MAIN_WINDOW),
                                PREVIEWS_EVENT,
                                previews,
                            );
                        }
                        mailx_core::CoreEvent::Outbox(change) => {
                            compose::emit_outbox(&handle, change);
                            let app = settings_handle.clone();
                            let outbox_id = change.outbox_id.clone();
                            notifications::send_result(
                                &handle,
                                change.clone(),
                                notification_accounts.clone(),
                                move || {
                                    let state = app.try_state::<state::DesktopState>()?;
                                    let (settings, locale) = state.notification_settings()?;
                                    // The subject is read only to show it, and only
                                    // when previews are on.
                                    let subject = settings
                                        .show_preview
                                        .then(|| state.core().ok()?.get_outbox(&outbox_id).ok())
                                        .flatten()
                                        .map(|item| item.subject);
                                    Some((settings, subject, locale))
                                },
                            );
                        }
                        mailx_core::CoreEvent::AccountRemoved(removed) => {
                            notification_accounts.retire(&removed.account_id);
                            if let Some(protocol) =
                                handle.try_state::<std::sync::Arc<body_protocol::BodyProtocol>>()
                            {
                                protocol.revoke_account(&removed.account_id);
                            }
                            compose::emit_account_removed(&handle, removed);
                        }
                        mailx_core::CoreEvent::DraftChanged(change) => {
                            compose::emit_draft(&handle, change);
                        }
                        mailx_core::CoreEvent::NewMail(mail) => {
                            let app = settings_handle.clone();
                            notifications::new_mail(
                                &handle,
                                mail.clone(),
                                notification_accounts.clone(),
                                move || {
                                    app.try_state::<state::DesktopState>()?
                                        .notification_settings()
                                },
                            );
                        }
                    });
                let state = state::DesktopState::new(data_dir, secrets, events);
                app.manage(state.clone());
                app.manage(compose::Lifecycle::default());
                app.manage(compose_files::DroppedFiles::default());
                app.manage(lifecycle::LifecycleState::default());
                app.manage(tray::TrayState::default());
                app.manage(updater::UpdaterState::default());
                updater::start_scheduler(app.handle());
                // Prepare the data directory before exposing business commands;
                // the task itself runs on Tauri's blocking pool.
                state.start_initialization();
                notifications::on_click(app.handle());
                window_chrome::create_main_window(app.handle())?;
                lifecycle::setup(app.handle());
                Ok(())
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_get_startup_status,
            commands::app_retry_startup,
            lifecycle::app_get_lifecycle,
            lifecycle::app_resolve_close,
            lifecycle::app_quit,
            lifecycle::app_cancel_quit,
            lifecycle::app_confirm_quit,
            lifecycle::app_desktop_ready,
            commands::accounts_list,
            commands::accounts_get,
            commands::accounts_create,
            commands::accounts_remove,
            commands::accounts_update_connection,
            commands::accounts_update_profile,
            commands::accounts_verify_connection,
            commands::accounts_cancel_verification,
            commands::accounts_verify_draft_connection,
            commands::accounts_cancel_draft_verification,
            commands::accounts_open_provider_help,
            commands::folders_list,
            commands::emails_list,
            commands::emails_get,
            commands::emails_fetch_body,
            commands::emails_request_previews,
            commands::emails_prepare_body,
            commands::emails_image_status,
            commands::emails_retry_images,
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
            commands::window_set_theme,
            compose::drafts_create,
            compose::drafts_create_from,
            compose::drafts_list,
            compose::drafts_get,
            compose::drafts_update,
            compose_files::attachments_add,
            compose_files::attachments_add_dropped,
            compose_files::attachments_remove,
            compose_files::attachments_policy,
            compose::drafts_delete,
            compose::outbox_queue_send,
            compose::outbox_list,
            compose::outbox_get,
            compose::outbox_cancel,
            compose::outbox_retry_now,
            compose::outbox_resolve_unknown,
            compose::composer_open,
            compose::composer_close,
            compose::composer_cancel_close,
            updater::updater_get_state,
            updater::updater_check,
            updater::updater_download,
            updater::updater_cancel,
            updater::updater_restart,
            updater::updater_open_link,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build the MailX desktop host")
        .run(|_app, _event| {
            // Explicit quit bypasses the main-window close preference and waits
            // for UI persistence and active sending before committing exit.
            #[cfg(not(feature = "body-webview-test"))]
            if let tauri::RunEvent::ExitRequested { api, .. } = &_event
                && !compose::is_exiting(_app)
            {
                api.prevent_exit();
                compose::request_quit(_app);
            }
            #[cfg(all(target_os = "macos", not(feature = "body-webview-test")))]
            if matches!(_event, tauri::RunEvent::Reopen { .. }) {
                window_chrome::focus_main_window(_app);
            }
            // Every window is gone and every composer has saved: install a
            // downloaded update now, and relaunch when the user asked to.
            #[cfg(not(feature = "body-webview-test"))]
            if matches!(_event, tauri::RunEvent::Exit) {
                updater::on_exit(_app);
            }
        });
}
