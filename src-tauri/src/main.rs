#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod config;
mod inspect;
mod merge;
mod model;
mod scan;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::AppState {
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_state,
            commands::get_plan,
            commands::get_suggestions,
            commands::path_exists,
            commands::get_log_path,
            commands::save_text_file,
            commands::scan_folder,
            commands::scan_files,
            commands::reload_table,
            commands::reload_group,
            commands::preview_source,
            commands::preview_merged,
            commands::run_preflight,
            commands::start_merge,
            commands::cancel_merge,
            commands::save_scheme,
            commands::open_scheme,
            commands::check_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
