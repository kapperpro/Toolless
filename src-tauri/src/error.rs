//! Einheitlicher Fehlertyp. Wird als Klartext ans Frontend serialisiert.

use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Datenbankfehler: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("Zwischenablage: {0}")]
    Clipboard(#[from] arboard::Error),
    #[error("Dateisystem: {0}")]
    Io(#[from] std::io::Error),
    #[error("Tauri: {0}")]
    Tauri(#[from] tauri::Error),
    #[error("Spotify: {0}")]
    Spotify(String),
    #[error("{0}")]
    Other(String),
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    pub fn other(msg: impl Into<String>) -> Self {
        Self::Other(msg.into())
    }
}

impl From<rspotify::ClientError> for AppError {
    fn from(err: rspotify::ClientError) -> Self {
        let raw = err.to_string();
        let hint = if raw.contains("404") {
            " – kein aktives Gerät. Öffne Spotify und starte kurz die Wiedergabe."
        } else if raw.contains("403") {
            " – die Wiedergabesteuerung erfordert Spotify Premium."
        } else if raw.contains("401") {
            " – Anmeldung abgelaufen. Bitte neu verbinden."
        } else {
            ""
        };
        Self::Spotify(format!("{raw}{hint}"))
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
