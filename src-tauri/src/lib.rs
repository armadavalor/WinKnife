//! Cleanify — modular Windows utility.
//!
//! Analyze first, never delete without consent, everything reversible.
//!
//! This is the crate root: it wires the module registry and the database into
//! Tauri's managed state and registers the commands. `main.rs` is only a shim
//! so the same entry point can be reused by the mobile target.

pub mod commands;
pub mod core;
pub mod modules;

use tauri::Manager;

use crate::core::db;
use crate::core::error::CoreError;

/// Build and run the desktop application.
///
/// # Panics
/// Panics if Tauri itself cannot start (no WebView2 runtime, broken bundle
/// context). There is no UI to report the failure in at that point, so failing
/// loudly is the only honest option.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            app.manage(modules::registry()?);

            let app_data_dir = app
                .path()
                .app_data_dir()
                .map_err(|err| CoreError::AppDataDir(format!("{err}")))?;
            let connection = db::open(&app_data_dir)?;
            app.manage(db::Database::new(connection));

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![commands::list_modules])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
