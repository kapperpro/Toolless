//! Einheitlicher Fehlertyp der Anwendung.

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Zwischenablage: {0}")]
    Clipboard(#[from] arboard::Error),
    #[error("Dateisystem: {0}")]
    Io(#[from] std::io::Error),
    #[error("Medien: {0}")]
    Media(String),
    #[error("{0}")]
    Other(String),
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    pub fn other(msg: impl Into<String>) -> Self {
        Self::Other(msg.into())
    }
}
