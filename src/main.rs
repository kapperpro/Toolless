//! Toolless – Werkzeugsammlung für den Desktop, vollständig in Rust (egui/eframe).
//!
//! Aufbau:
//! - `app`       Hauptfenster, Navigation, Hintergrundbetrieb
//! - `ui/*`      Oberfläche je Modul
//! - `services/*` Fachlogik (Spotify, Zwischenablage, Notizen, Starter, …)
//! - `tray`      System-Tray
//! - `platform`  Fenster verstecken/anzeigen
//! - `theme`     Farben und Bausteine im Windows-Stil, anpassbar
//! - `settings`  Benutzereinstellungen (Design, Verhalten)
//! - `db`, `crypto` verschlüsselter Datenspeicher
//! - `error`

// Unter Windows im Release-Build kein zusätzliches Konsolenfenster öffnen.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod crypto;
mod db;
mod error;
mod i18n;
mod i18n_table;
mod platform;
mod services;
mod settings;
mod theme;
mod tray;
mod ui;

use eframe::egui;

const ICON_RGBA: &[u8] = include_bytes!("../assets/icon.rgba");
const ICON_SIZE: u32 = 64;

fn main() -> eframe::Result<()> {
    // Nur eine Instanz: ein zweiter Start beendet sich sofort wieder.
    let instance = single_instance::SingleInstance::new("xyz.toolless.desktop").ok();
    if instance.as_ref().is_some_and(|i| !i.is_single()) {
        return Ok(());
    }

    // Per Autostart gestartet → still im Tray bleiben.
    let minimized = std::env::args().any(|arg| arg == services::autostart::MINIMIZED_FLAG);
    let visible = !minimized;

    let icon = egui::IconData {
        rgba: ICON_RGBA.to_vec(),
        width: ICON_SIZE,
        height: ICON_SIZE,
    };
    let viewport = egui::ViewportBuilder::default()
        .with_title("Toolless")
        .with_inner_size([1040.0, 700.0])
        .with_min_inner_size([860.0, 560.0])
        .with_decorations(false)
        .with_visible(visible)
        .with_icon(icon);
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Toolless",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, visible)?))),
    )
}
