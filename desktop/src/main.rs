#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod secret_store;
mod state;

use std::sync::Arc;
use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .map_err(|_| mailx_core::AppError::new(mailx_core::ErrorCode::StorageUnavailable));
            let secrets = Arc::new(secret_store::SystemSecretStore::new(
                app.config().identifier.clone(),
            ));
            app.manage(state::DesktopState::new(data_dir, secrets));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::accounts_list,
            commands::accounts_get,
            commands::accounts_create,
            commands::accounts_update,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run the MailX desktop host");
}
