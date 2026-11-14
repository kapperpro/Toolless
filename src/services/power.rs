//! „Anti PC Power Off“: blockiert Ruhezustand und Bildschirm-Aus, solange aktiv.

use std::{
    sync::{mpsc, Mutex},
    thread,
};

use crate::error::{AppError, AppResult};

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

    pub fn set(&self, enabled: bool) -> AppResult<()> {
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

    /// Schaltet um und liefert den neuen Zustand.
    pub fn toggle(&self) -> AppResult<bool> {
        let target = !self.is_active();
        self.set(target)?;
        Ok(target)
    }
}
