//! ADB Vizzard backend.
//!
//! See `docs/DESIGN.md` for the design. The two load-bearing decisions: artifact bytes
//! come straight from blob storage via batched signed URIs (§5.2), and finished
//! runs are cached forever because their artifacts are immutable.

pub mod cache;
pub mod client;
pub mod commands;
pub mod comparator;
pub mod creds;
pub mod error;
pub mod exporter;
pub mod fetcher;
pub mod model;
pub mod parser;
pub mod runref;
pub mod settings;
pub mod state;

use tauri::Manager;

use crate::settings::SettingsStore;
use crate::state::AppState;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let handle = app.handle();

            // Settings are the only thing written to disk. Artifacts stay in
            // memory for the session (see cache.rs for why).
            let config_dir = handle
                .path()
                .app_config_dir()
                .map_err(|e| format!("could not resolve the config directory: {e}"))?;

            std::fs::create_dir_all(&config_dir).ok();

            let http = client::build_http_client().map_err(|e| e.to_string())?;
            let cwd = std::env::current_dir().unwrap_or_else(|_| config_dir.clone());

            let state = AppState::new(http, SettingsStore::new(config_dir), cwd);

            // Apply the saved theme before the window is shown, so there is no
            // flash of the wrong palette.
            if let Some(window) = handle.get_webview_window("main") {
                let theme = match state
                    .settings
                    .try_read()
                    .map(|s| s.theme.clone())
                    .unwrap_or_default()
                    .as_str()
                {
                    "light" => Some(tauri::Theme::Light),
                    "dark" => Some(tauri::Theme::Dark),
                    _ => None,
                };
                let _ = window.set_theme(theme);
            }

            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::boot_info,
            commands::connect,
            commands::disconnect,
            commands::reveal_token,
            commands::forget_credentials,
            commands::load_side,
            commands::load_local_side,
            commands::list_local_parents,
            commands::cancel_load,
            commands::clear_side,
            commands::build_comparison,
            commands::build_curves,
            commands::export,
            commands::comparison_markdown,
            commands::swap_sides,
            commands::save_settings,
            commands::cache_stats,
            commands::clear_cache,
            commands::parse_run_ref,
            commands::metric_catalog,
        ])
        .run(tauri::generate_context!())
        .expect("error while running ADB Vizzard");
}
