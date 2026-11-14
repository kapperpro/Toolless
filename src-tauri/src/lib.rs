//! Toolless – Einstiegspunkt des Rust-Backends.
//!
//! Aufbau:
//! - `commands/*`  Tauri-Commands je Funktionsbereich (Spotify, Clipboard, …)
//! - `db`          SQLite-Persistenz
//! - `tray`        System-Tray und Hintergrundbetrieb
//! - `window`      Fenster-Hilfsfunktionen
//! - `error`       gemeinsamer Fehlertyp

mod commands;
mod db;
mod error;
mod tray;
mod window;

use tauri::{Manager, WindowEvent};

use commands::{autostart, clipboard, launcher, power, spotify, web};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        // Zweiter Start holt nur das bestehende Fenster nach vorn.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            window::show_main(app);
        }))
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_opener::init())
        .manage(power::PowerState::default())
        .manage(spotify::SpotifyState::default())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            app.manage(db::Db::open(&data_dir.join("toolless.sqlite"))?);

            tray::create(app.handle())?;

            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move { spotify::restore(&handle).await });

            // Per Autostart gestartet → still im Tray bleiben.
            let minimized = std::env::args().any(|arg| arg == autostart::MINIMIZED_FLAG);
            if !minimized {
                window::show_main(app.handle());
            }
            Ok(())
        })
        // Schließen des Hauptfensters versteckt es nur – die App läuft im Tray weiter.
        .on_window_event(|window, event| {
            if window.label() == window::MAIN_WINDOW {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            clipboard::clipboard_read,
            clipboard::clipboard_write,
            clipboard::clip_list,
            clipboard::clip_save,
            clipboard::clip_update,
            clipboard::clip_delete,
            clipboard::clip_copy,
            autostart::autostart_is_enabled,
            autostart::autostart_set,
            power::power_is_active,
            power::power_set,
            launcher::launcher_scan,
            launcher::launcher_launch,
            spotify::spotify_status,
            spotify::spotify_login,
            spotify::spotify_logout,
            spotify::spotify_playback,
            spotify::spotify_toggle,
            spotify::spotify_next,
            spotify::spotify_previous,
            spotify::spotify_set_volume,
            web::open_earweb_window,
        ])
        .build(tauri::generate_context!())
        .expect("Toolless konnte nicht gestartet werden");

    app.run(|_app, _event| {
        // macOS: Klick auf das Dock-Icon zeigt das versteckte Fenster wieder an.
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen { .. } = _event {
            window::show_main(_app);
        }
    });
}
