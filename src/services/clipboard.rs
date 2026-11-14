//! Erweiterter Clipboard-Manager: System-Zwischenablage (arboard) + SQLite-Archiv.

use arboard::Clipboard;
use rusqlite::{params, Row};

use crate::{
    db::Db,
    error::{AppError, AppResult},
};

#[derive(Debug, Clone)]
pub struct ClipItem {
    pub id: i64,
    pub title: String,
    pub category: String,
    pub content: String,
}

const COLUMNS: &str = "id, title, category, content";

fn from_row(row: &Row<'_>) -> rusqlite::Result<ClipItem> {
    Ok(ClipItem {
        id: row.get(0)?,
        title: row.get(1)?,
        category: row.get(2)?,
        content: row.get(3)?,
    })
}

/// Leerer Titel → die ersten Zeichen des Inhalts dienen als Titel.
fn normalize_title(title: &str, content: &str) -> String {
    let title = title.trim();
    if !title.is_empty() {
        return title.to_owned();
    }
    let first_line = content
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("Ohne Titel");
    first_line.trim().chars().take(48).collect()
}

/// Liest den aktuellen Textinhalt der System-Zwischenablage.
pub fn read_system() -> AppResult<String> {
    match Clipboard::new()?.get_text() {
        Ok(text) => Ok(text),
        // Leere Zwischenablage oder kein Text (z. B. ein Bild) ist kein Fehler.
        Err(arboard::Error::ContentNotAvailable) => Ok(String::new()),
        Err(err) => Err(err.into()),
    }
}

/// Schreibt Text in die System-Zwischenablage.
pub fn write_system(text: &str) -> AppResult<()> {
    Clipboard::new()?.set_text(text.to_owned())?;
    Ok(())
}

pub fn list(db: &Db) -> AppResult<Vec<ClipItem>> {
    db.with(|c| {
        let mut stmt = c.prepare(&format!(
            "SELECT {COLUMNS} FROM clipboard_items ORDER BY id DESC"
        ))?;
        let rows = stmt.query_map([], from_row)?;
        rows.collect()
    })
}

pub fn save(db: &Db, title: &str, category: &str, content: &str) -> AppResult<ClipItem> {
    if content.is_empty() {
        return Err(AppError::other("Die Zwischenablage enthält keinen Text."));
    }
    let title = normalize_title(title, content);
    let category = category.trim().to_owned();
    let id = db.with(|c| {
        c.execute(
            "INSERT INTO clipboard_items (title, category, content) VALUES (?1, ?2, ?3)",
            params![title, category, content],
        )?;
        Ok(c.last_insert_rowid())
    })?;
    Ok(ClipItem {
        id,
        title,
        category,
        content: content.to_owned(),
    })
}

pub fn update(db: &Db, item: &ClipItem) -> AppResult<ClipItem> {
    let updated = ClipItem {
        id: item.id,
        title: normalize_title(&item.title, &item.content),
        category: item.category.trim().to_owned(),
        content: item.content.clone(),
    };
    let changed = db.with(|c| {
        c.execute(
            "UPDATE clipboard_items SET title = ?1, category = ?2, content = ?3 WHERE id = ?4",
            params![updated.title, updated.category, updated.content, updated.id],
        )
    })?;
    if changed == 0 {
        return Err(AppError::other("Eintrag nicht gefunden."));
    }
    Ok(updated)
}

pub fn delete(db: &Db, id: i64) -> AppResult<()> {
    db.with(|c| {
        c.execute("DELETE FROM clipboard_items WHERE id = ?1", params![id])
            .map(|_| ())
    })
}
