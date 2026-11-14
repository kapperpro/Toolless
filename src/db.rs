//! SQLite-Zugriff: Schema, Verbindungs-Wrapper und Key/Value-Einstellungen.

use std::{path::Path, sync::Mutex};

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{AppError, AppResult};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS clipboard_items (
    id         INTEGER PRIMARY KEY AUTOINCREMENT,
    title      TEXT NOT NULL,
    category   TEXT NOT NULL DEFAULT '',
    content    TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_clipboard_category ON clipboard_items(category);
CREATE TABLE IF NOT EXISTS settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

/// Thread-sicherer Wrapper um die SQLite-Verbindung (als Tauri-State verwaltet).
pub struct Db(Mutex<Connection>);

impl Db {
    pub fn open(path: &Path) -> AppResult<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self(Mutex::new(conn)))
    }

    /// Führt `f` mit exklusivem Zugriff auf die Verbindung aus.
    pub fn with<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> AppResult<T> {
        let conn = self
            .0
            .lock()
            .map_err(|_| AppError::other("Datenbank-Lock ist nicht verfügbar"))?;
        Ok(f(&conn)?)
    }

    pub fn get_setting(&self, key: &str) -> AppResult<Option<String>> {
        self.with(|c| {
            c.query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |r| r.get(0),
            )
            .optional()
        })
    }

    pub fn set_setting(&self, key: &str, value: &str) -> AppResult<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map(|_| ())
        })
    }

    pub fn delete_setting(&self, key: &str) -> AppResult<()> {
        self.with(|c| {
            c.execute("DELETE FROM settings WHERE key = ?1", params![key])
                .map(|_| ())
        })
    }
}
