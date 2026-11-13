#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod attachment_save;
mod autostart;
mod body_protocol;
mod commands;
mod compose;
mod compose_files;
mod lifecycle;
mod locale;
mod mcp;
mod mcp_sidecar;
mod notifications;
mod secret_split;
mod secret_store;
mod secret_vault;
mod state;
mod storage_location;
mod tray;
#[cfg(target_os = "linux")]
mod tray_support;
mod updater;
mod window_access;
mod window_chrome;

use std::sync::Arc;
use tauri::{Emitter, EventTarget, Manager};

/// Pushed to the main window whenever an account's sync state changes.
const SYNC_STATE_EVENT: &str = "mailx://sync-state";
/// Pushed after a read / starred write-back round, for refreshing markers and
/// telling the user a star stayed on this device.
const FLAG_EVENT: &str = "mailx://flags";
const DELETIONS_EVENT: &str = "mailx://deletions";
const MAIL_STATE_EVENT: &str = "mailx://mail-state-changed";

/// List previews fetched for rows the main window asked about.
const PREVIEWS_EVENT: &str = "mailx://email-previews";

/// An account's local sent copies changed; Sent lists refetch.
const SENT_COPIES_EVENT: &str = "mailx://sent-copies";

fn main() {
    let builder = tauri::Builder::default()
        .manage(autostart::StartupIntent::default())
        .manage(autostart::foreground_environment(tauri::Env::default()));
    // AppKit otherwise activates the app even when its initial window is
    // hidden and unfocused. A visible/manual launch still focuses explicitly.
    #[cfg(target_os = "macos")]
    let builder =
        builder.activate_ignoring_other_apps(!autostart::is_login_launch(std::env::args()));
    // Registered first, as the plugin requires. A second launch hands over to the
    // running instance and exits, so it never reaches the data-directory lock.
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
        if !autostart::is_login_launch(args) {
            window_chrome::focus_main_window(app);
        }
    }));
    let builder = builder.plugin(tauri_plugin_notification::init());
    let builder = builder.plugin(tauri_plugin_updater::Builder::new().build());
    let builder = builder
        .plugin(tauri_plugin_dialog::init())
        .append_invoke_initialization_script(include_str!("subframe_guard.js"))
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        );
    let builder = builder.plugin(window_chrome::window_state_plugin());
    // Packaged builds only: development keeps the inspector and reload.
    #[cfg(windows)]
    let builder = if cfg!(debug_assertions) {
        builder
    } else {
        builder.plugin(window_chrome::browser_defaults_plugin())
    };
    body_protocol::register(builder)
        .setup(|app| {
            let storage = app
                .path()
                .app_data_dir()
                .map_err(|_| mailx_core::AppError::new(mailx_core::ErrorCode::StorageUnavailable))
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
            let notification_accounts = Arc::new(notifications::AccountNotifications::default());
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
                    mailx_core::CoreEvent::MailStateChanged(change) => {
                        let _ = handle.emit_to(
                            EventTarget::webview_window(window_chrome::MAIN_WINDOW),
                            MAIL_STATE_EVENT,
                            change,
                        );
                    }
                    mailx_core::CoreEvent::Deletions(deleted) => {
                        let _ = handle.emit_to(
                            EventTarget::webview_window(window_chrome::MAIN_WINDOW),
                            DELETIONS_EVENT,
                            deleted,
                        );
                    }
                    mailx_core::CoreEvent::Previews(previews) => {
                        let _ = handle.emit_to(
                            EventTarget::webview_window(window_chrome::MAIN_WINDOW),
                            PREVIEWS_EVENT,
                            previews,
                        );
                    }
                    mailx_core::CoreEvent::SentCopies(changed) => {
                        let _ = handle.emit_to(
                            EventTarget::webview_window(window_chrome::MAIN_WINDOW),
                            SENT_COPIES_EVENT,
                            changed,
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
            app.manage(autostart::AutostartState::new(app.handle()));
            updater::start_scheduler(app.handle());
            // Prepare the data directory before exposing business commands;
            // the task itself runs on Tauri's blocking pool.
            // Registered before initialization so the ready hook fires; the
            // single-instance plugin has already let this process live.
            mcp::setup(app.handle(), &state);
            state.start_initialization();
            notifications::on_click(app.handle());
            let login_launch = autostart::is_login_launch(std::env::args());
            window_chrome::create_main_window(
                app.handle(),
                !login_launch || autostart::user_activated(app.handle()),
            )?;
            lifecycle::setup(app.handle());
            if login_launch {
                autostart::prepare_startup(app.handle());
            }
            Ok(())
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
            autostart::autostart_get,
            autostart::autostart_set,
            commands::accounts_list,
            commands::accounts_get,
            commands::accounts_create,
            commands::accounts_remove,
            commands::accounts_update_connection,
            commands::accounts_update_profile,
            commands::accounts_oauth_reauthorize,
            commands::accounts_oauth_begin,
            commands::accounts_oauth_finish,
            commands::accounts_oauth_confirm_mailbox,
            commands::accounts_oauth_create,
            commands::accounts_oauth_reauthorize_finish,
            commands::accounts_oauth_cancel,
            commands::accounts_verify_connection,
            commands::accounts_cancel_verification,
            commands::accounts_verify_draft_connection,
            commands::accounts_cancel_draft_verification,
            commands::accounts_open_provider_help,
            commands::folders_list,
            commands::emails_list,
            commands::emails_overview,
            commands::emails_get,
            commands::emails_get_conversation,
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
            commands::emails_delete,
            commands::emails_undo_delete,
            commands::emails_archive,
            commands::emails_unarchive,
            commands::emails_set_flags,
            commands::emails_select,
            commands::contacts_list_aggregations,
            commands::contacts_set_aggregation,
            commands::emails_undo_archive,
            commands::attachments_save,
            commands::avatars_get,
            commands::image_cache_clear,
            commands::translation_status,
            commands::translation_translate,
            commands::translation_cancel,
            commands::translation_clear,
            commands::translation_clear_all,
            commands::translation_cache_stats,
            commands::translation_keys,
            commands::translation_set_key,
            commands::translation_remove_key,
            commands::translation_proxy_password,
            commands::translation_set_proxy_password,
            commands::translation_remove_proxy_password,
            commands::translation_test,
            commands::ai_state,
            commands::ai_save_connection,
            commands::ai_remove_connection,
            commands::ai_set_default_model,
            commands::ai_set_translation_model,
            commands::ai_set_route,
            commands::ai_set_proxy_password,
            commands::ai_remove_proxy_password,
            commands::ai_catalog,
            commands::ai_list_models,
            commands::ai_test,
            commands::ai_provider_icon,
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
            compose::drafts_prepare_body,
            compose::drafts_update,
            compose_files::attachments_add,
            compose_files::attachments_add_dropped,
            compose_files::attachments_remove,
            compose_files::attachments_policy,
            compose_files::inline_images_add,
            compose_files::inline_images_add_bytes,
            compose_files::inline_image_preview,
            compose::drafts_delete,
            compose::outbox_queue_send,
            compose::outbox_list,
            compose::outbox_get,
            compose::outbox_cancel,
            compose::outbox_retry_now,
            compose::outbox_resolve_unknown,
            compose::outbox_remove_local_copy,
            compose::composer_open,
            compose::composer_close,
            compose::composer_cancel_close,
            updater::updater_get_state,
            updater::updater_check,
            updater::updater_download,
            updater::updater_cancel,
            updater::updater_restart,
            updater::updater_open_link,
            mcp::mcp_get_status,
            mcp::mcp_set_enabled,
            mcp::mcp_client_config,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build the MailX desktop host")
        .run(|_app, _event| {
            // Explicit quit bypasses the main-window close preference and waits
            // for UI persistence and active sending before committing exit.
            if let tauri::RunEvent::ExitRequested { api, .. } = &_event
                && !compose::is_exiting(_app)
            {
                api.prevent_exit();
                compose::request_quit(_app);
            }
            #[cfg(target_os = "macos")]
            if matches!(_event, tauri::RunEvent::Reopen { .. }) {
                window_chrome::focus_main_window(_app);
            }
            // Every window is gone and every composer has saved: install a
            // downloaded update now, and relaunch when the user asked to.
            if matches!(_event, tauri::RunEvent::Exit) {
                // Close the local MCP endpoint and remove its discovery file
                // before an update installer or relaunch takes over.
                mcp::shutdown(_app);
                updater::on_exit(_app);
            }
        });
}
