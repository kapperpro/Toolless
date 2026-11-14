//! System-Tray (Menüleisten-Icon): Fenster öffnen, Schnellsteuerung, Beenden.
//!
//! Die Callbacks laufen unabhängig von der egui-Schleife – sie funktionieren
//! also auch, während das Hauptfenster versteckt ist.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::Sender,
        Arc,
    },
    thread,
    time::Duration,
};

use eframe::egui;
use tray_icon::{
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

use crate::{
    error::{AppError, AppResult},
    platform::NativeWindow,
    services::{power::PowerState, spotify::Command},
};

const ICON_RGBA: &[u8] = include_bytes!("../assets/icon.rgba");
const ICON_SIZE: u32 = 64;

/// Alles, was die Tray-Callbacks brauchen (muss `Send + Sync` sein).
pub struct TrayDeps {
    pub ctx: egui::Context,
    pub window: NativeWindow,
    pub power: Arc<PowerState>,
    pub spotify: Sender<Command>,
    pub quit: Arc<AtomicBool>,
}

pub struct Tray {
    /// Muss am Leben bleiben, sonst verschwindet das Icon.
    _icon: TrayIcon,
    /// Häkchen wird von der App mit dem echten Zustand synchronisiert.
    pub keep_awake: CheckMenuItem,
}

fn tray_err(err: impl std::fmt::Display) -> AppError {
    AppError::other(format!("System-Tray: {err}"))
}

pub fn create(deps: TrayDeps) -> AppResult<Tray> {
    let show = MenuItem::with_id("show", "Toolless öffnen", true, None);
    let play_pause = MenuItem::with_id("spotify_toggle", "Spotify: Play / Pause", true, None);
    let next = MenuItem::with_id("spotify_next", "Spotify: Nächster Titel", true, None);
    let previous = MenuItem::with_id("spotify_previous", "Spotify: Vorheriger Titel", true, None);
    let keep_awake =
        CheckMenuItem::with_id("keep_awake", "Ruhezustand blockieren", true, false, None);
    let quit = MenuItem::with_id("quit", "Toolless beenden", true, None);

    let menu = Menu::new();
    menu.append_items(&[
        &show,
        &PredefinedMenuItem::separator(),
        &play_pause,
        &next,
        &previous,
        &PredefinedMenuItem::separator(),
        &keep_awake,
        &PredefinedMenuItem::separator(),
        &quit,
    ])
    .map_err(tray_err)?;

    let icon = Icon::from_rgba(ICON_RGBA.to_vec(), ICON_SIZE, ICON_SIZE).map_err(tray_err)?;
    let tray_icon = TrayIconBuilder::new()
        .with_tooltip("Toolless")
        .with_icon(icon)
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .build()
        .map_err(tray_err)?;

    // Linksklick auf das Icon öffnet das Hauptfenster, Rechtsklick das Menü.
    let (ctx, window) = (deps.ctx.clone(), deps.window.clone());
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            window.show(&ctx);
        }
    }));

    MenuEvent::set_event_handler(Some(move |event: MenuEvent| match event.id.as_ref() {
        "show" => deps.window.show(&deps.ctx),
        "spotify_toggle" => {
            let _ = deps.spotify.send(Command::Toggle);
        }
        "spotify_next" => {
            let _ = deps.spotify.send(Command::Next);
        }
        "spotify_previous" => {
            let _ = deps.spotify.send(Command::Previous);
        }
        "keep_awake" => {
            if let Err(err) = deps.power.toggle() {
                eprintln!("Tray/Keep-Awake: {err}");
            }
            deps.ctx.request_repaint();
        }
        "quit" => {
            // Sauberer Weg: Fenster einblenden, die App schließt sich im nächsten Frame.
            deps.quit.store(true, Ordering::Relaxed);
            deps.window.show(&deps.ctx);
            // Sicherheitsnetz, falls kein Frame mehr gezeichnet wird.
            thread::spawn(|| {
                thread::sleep(Duration::from_millis(1500));
                std::process::exit(0);
            });
        }
        _ => {}
    }));

    Ok(Tray {
        _icon: tray_icon,
        keep_awake,
    })
}
