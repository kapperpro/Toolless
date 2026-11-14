//! „Anti PC Power Off“: blockiert Ruhezustand und Bildschirm-Aus, solange aktiv.

use std::{
    sync::{mpsc, Mutex},
    thread,
};

use tauri::{AppHandle, Emitter, Manager, State};

use crate::{
    error::{AppError, AppResult},
    tray::TrayState,
};

/// Event an das Frontend, wenn der Zustand (z. B. über das Tray-Menü) wechselt.
pub const EVENT_CHANGED: &str = "keep-awake-changed";

/// Hält die Wach-Sperre in einem eigenen Thread.
///
/// Die Sperre des Betriebssystems ist teils an den erzeugenden Thread gebunden,
/// deshalb wird sie dort erzeugt *und* wieder freigegeben. Der Sender dient als
/// Stopp-Signal: Wird er verworfen, endet der Thread und gibt die Sperre frei.
#[derive(Default)]
pub struct PowerState {
    stop: Mutex<Option<mpsc::Sender<()>>>,
}

impl PowerState {
    pub fn is_active(&self) -> bool {
        self.stop
            .lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false)
    }

    fn set(&self, enabled: bool) -> AppResult<()> {
        let mut stop = self
            .stop
            .lock()
            .map_err(|_| AppError::other("Power-Status ist nicht verfügbar"))?;

        if !enabled {
            stop.take(); // Sender verwerfen → Thread beendet sich.
            return Ok(());
        }
        if stop.is_some() {
            return Ok(());
        }

        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
        thread::Builder::new()
            .name("keep-awake".into())
            .spawn(move || {
                let guard = keepawake::Builder::default()
                    .display(true)
                    .idle(true)
                    .sleep(true)
                    .reason("Toolless: Ruhezustand blockiert")
                    .app_name("Toolless")
                    .app_reverse_domain("xyz.toolless.desktop")
                    .create();
                match guard {
                    Ok(_guard) => {
                        let _ = ready_tx.send(Ok(()));
                        // Blockiert, bis der Sender verworfen wird.
                        let _ = stop_rx.recv();
                    }
                    Err(err) => {
                        let _ = ready_tx.send(Err(err.to_string()));
                    }
                }
            })?;

        match ready_rx.recv() {
            Ok(Ok(())) => {
                *stop = Some(stop_tx);
                Ok(())
            }
            Ok(Err(msg)) => Err(AppError::other(format!(
                "Ruhezustand konnte nicht blockiert werden: {msg}"
            ))),
            Err(_) => Err(AppError::other(
                "Keep-Awake-Thread wurde unerwartet beendet",
            )),
        }
    }
}

/// Schaltet die Sperre und synchronisiert Tray-Menü und Frontend.
pub fn apply(app: &AppHandle, enabled: bool) -> AppResult<bool> {
    let state = app.state::<PowerState>();
    let result = state.set(enabled);
    let active = state.is_active();

    if let Some(tray) = app.try_state::<TrayState>() {
        let _ = tray.keep_awake.set_checked(active);
    }
    let _ = app.emit(EVENT_CHANGED, active);

    result.map(|_| active)
}

#[tauri::command]
pub fn power_is_active(state: State<'_, PowerState>) -> bool {
    state.is_active()
}

#[tauri::command]
pub fn power_set(app: AppHandle, enabled: bool) -> AppResult<bool> {
    apply(&app, enabled)
}
