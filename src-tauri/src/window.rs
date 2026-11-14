//! Hilfsfunktionen rund um das Hauptfenster.

use tauri::{AppHandle, Manager};

pub const MAIN_WINDOW: &str = "main";

/// Holt das Hauptfenster aus dem Hintergrund zurück in den Vordergrund.
pub fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}
