//! System-Tray (Menüleisten-Icon): Fenster öffnen, Schnellsteuerung, Beenden.

use tauri::{
    menu::{CheckMenuItem, CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Wry,
};

use crate::{
    commands::{power, spotify},
    error::AppResult,
    window::show_main,
};

/// Handles auf Menüeinträge, deren Zustand von außen aktualisiert wird.
pub struct TrayState {
    pub keep_awake: CheckMenuItem<Wry>,
}

#[derive(Clone, Copy)]
enum SpotifyAction {
    Toggle,
    Next,
    Previous,
}

fn spotify_action(app: &AppHandle, action: SpotifyAction) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app.state::<spotify::SpotifyState>();
        let result = match action {
            SpotifyAction::Toggle => spotify::toggle(&state).await,
            SpotifyAction::Next => spotify::next(&state).await,
            SpotifyAction::Previous => spotify::previous(&state).await,
        };
        if let Err(err) = result {
            eprintln!("Tray/Spotify: {err}");
        }
    });
}

pub fn create(app: &AppHandle) -> AppResult<()> {
    let show = MenuItemBuilder::with_id("show", "Toolless öffnen").build(app)?;
    let play_pause =
        MenuItemBuilder::with_id("spotify_toggle", "Spotify: Play / Pause").build(app)?;
    let next = MenuItemBuilder::with_id("spotify_next", "Spotify: Nächster Titel").build(app)?;
    let previous =
        MenuItemBuilder::with_id("spotify_previous", "Spotify: Vorheriger Titel").build(app)?;
    let keep_awake = CheckMenuItemBuilder::with_id("keep_awake", "Ruhezustand blockieren")
        .checked(false)
        .build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Toolless beenden").build(app)?;

    let menu = MenuBuilder::new(app)
        .item(&show)
        .separator()
        .item(&play_pause)
        .item(&next)
        .item(&previous)
        .separator()
        .item(&keep_awake)
        .separator()
        .item(&quit)
        .build()?;

    let mut tray = TrayIconBuilder::with_id("main-tray")
        .tooltip("Toolless")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main(app),
            "spotify_toggle" => spotify_action(app, SpotifyAction::Toggle),
            "spotify_next" => spotify_action(app, SpotifyAction::Next),
            "spotify_previous" => spotify_action(app, SpotifyAction::Previous),
            "keep_awake" => {
                let enable = !app.state::<power::PowerState>().is_active();
                if let Err(err) = power::apply(app, enable) {
                    eprintln!("Tray/Keep-Awake: {err}");
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // Linksklick auf das Icon öffnet das Hauptfenster, Rechtsklick das Menü.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;

    app.manage(TrayState { keep_awake });
    Ok(())
}
