//! Cleanify — modular Windows utility.
//!
//! Analyze first, never delete without consent, everything reversible.

pub mod core;

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
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
