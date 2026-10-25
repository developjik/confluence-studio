// Integration layer: module tree, command registration and app setup.
// Command/event contract: docs/ARCHITECTURE.md § "Tauri command contract".

pub mod commands;
pub mod confluence;
pub mod credentials;
pub mod damon;
pub mod jobs;
pub mod state;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let state = std::sync::Arc::new(state::AppState::init(app.handle().clone())?);
            // Background scheduler (30s tick) — see commands::spawn_scheduler.
            commands::spawn_scheduler(state.clone());
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // connection
            commands::save_connection,
            commands::get_connection,
            commands::test_connection,
            commands::clear_connection,
            // spaces / pages
            commands::list_spaces,
            commands::get_page_tree,
            commands::get_page,
            commands::create_page,
            commands::update_page,
            commands::delete_page,
            // jobs / scheduling
            commands::list_jobs,
            commands::save_job,
            commands::delete_job,
            commands::run_job,
            commands::cancel_job_run,
            commands::list_job_runs,
            // damon
            commands::damon_status,
            commands::damon_list_sessions,
            commands::damon_create_session,
            commands::damon_prompt,
            commands::damon_cancel_turn,
            commands::damon_permission_respond,
            commands::damon_delete_session,
            commands::damon_rename_session
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
