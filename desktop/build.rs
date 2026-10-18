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
            "sync_states",
            "window_get_chrome",
            "window_perform",
        ]),
    ))
    .expect("failed to prepare the MailX desktop host");
}
