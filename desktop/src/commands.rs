use serde::Serialize;

/// Development-only metadata; this is not the future mail API.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CoreInfo {
    core_version: &'static str,
}

#[tauri::command]
pub(crate) fn core_info() -> CoreInfo {
    CoreInfo {
        core_version: mailx_core::VERSION,
    }
}
