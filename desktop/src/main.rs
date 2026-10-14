#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![commands::core_info])
        .run(tauri::generate_context!())
        .expect("failed to run the MailX desktop host");
}
