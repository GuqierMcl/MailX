fn main() {
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "app_get_startup_status",
            "app_retry_startup",
            "accounts_list",
            "accounts_get",
            "accounts_create",
            "accounts_update",
            "folders_list",
            "emails_list",
            "emails_get",
            "sync_states",
            "settings_get",
            "settings_update",
            "window_get_chrome",
            "window_perform",
        ]),
    ))
    .expect("failed to prepare the MailX desktop host");
}
