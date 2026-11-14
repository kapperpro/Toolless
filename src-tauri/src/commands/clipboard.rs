//! Erweiterter Clipboard-Manager: System-Zwischenablage (arboard) + SQLite-Archiv.

use arboard::Clipboard;
use rusqlite::{params, Row};
use serde::Serialize;
use tauri::State;

use crate::{
    db::Db,
    error::{AppError, AppResult},
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipItem {
    id: i64,
    title: String,
    category: String,
    content: String,
    created_at: String,
}

impl ClipItem {
    const COLUMNS: &'static str = "id, title, category, content, created_at";

    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            title: row.get(1)?,
            category: row.get(2)?,
            content: row.get(3)?,
            created_at: row.get(4)?,
        })
    }
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

fn read_system_clipboard() -> AppResult<String> {
    match Clipboard::new()?.get_text() {
        Ok(text) => Ok(text),
        // Leere Zwischenablage oder kein Text (z. B. ein Bild) ist kein Fehler.
        Err(arboard::Error::ContentNotAvailable) => Ok(String::new()),
        Err(err) => Err(err.into()),
    }
}

fn write_system_clipboard(text: &str) -> AppResult<()> {
    Clipboard::new()?.set_text(text.to_owned())?;
    Ok(())
}

fn find(db: &Db, id: i64) -> AppResult<ClipItem> {
    db.with(|c| {
        c.query_row(
            &format!(
                "SELECT {} FROM clipboard_items WHERE id = ?1",
                ClipItem::COLUMNS
            ),
            params![id],
            ClipItem::from_row,
        )
    })
}

/// Liest den aktuellen Textinhalt der System-Zwischenablage.
#[tauri::command]
pub fn clipboard_read() -> AppResult<String> {
    read_system_clipboard()
}

/// Schreibt beliebigen Text in die System-Zwischenablage.
#[tauri::command]
pub fn clipboard_write(text: String) -> AppResult<()> {
    write_system_clipboard(&text)
}

#[tauri::command]
pub fn clip_list(db: State<'_, Db>) -> AppResult<Vec<ClipItem>> {
    db.with(|c| {
        let mut stmt = c.prepare(&format!(
            "SELECT {} FROM clipboard_items ORDER BY id DESC",
            ClipItem::COLUMNS
        ))?;
        let rows = stmt.query_map([], ClipItem::from_row)?;
        rows.collect()
    })
}

/// Speichert einen Eintrag. Ohne `content` wird die aktuelle Zwischenablage übernommen.
#[tauri::command]
pub fn clip_save(
    db: State<'_, Db>,
    title: String,
    category: String,
    content: Option<String>,
) -> AppResult<ClipItem> {
    let content = match content {
        Some(content) => content,
        None => read_system_clipboard()?,
    };
    if content.is_empty() {
        return Err(AppError::other("Die Zwischenablage enthält keinen Text."));
    }
    let title = normalize_title(&title, &content);
    let id = db.with(|c| {
        c.execute(
            "INSERT INTO clipboard_items (title, category, content) VALUES (?1, ?2, ?3)",
            params![title, category.trim(), content],
        )?;
        Ok(c.last_insert_rowid())
    })?;
    find(&db, id)
}

#[tauri::command]
pub fn clip_update(
    db: State<'_, Db>,
    id: i64,
    title: String,
    category: String,
    content: String,
) -> AppResult<ClipItem> {
    let title = normalize_title(&title, &content);
    let changed = db.with(|c| {
        c.execute(
            "UPDATE clipboard_items SET title = ?1, category = ?2, content = ?3 WHERE id = ?4",
            params![title, category.trim(), content, id],
        )
    })?;
    if changed == 0 {
        return Err(AppError::other("Eintrag nicht gefunden."));
    }
    find(&db, id)
}

#[tauri::command]
pub fn clip_delete(db: State<'_, Db>, id: i64) -> AppResult<()> {
    db.with(|c| {
        c.execute("DELETE FROM clipboard_items WHERE id = ?1", params![id])
            .map(|_| ())
    })
}

/// Befördert einen gespeicherten Eintrag zurück in die System-Zwischenablage.
#[tauri::command]
pub fn clip_copy(db: State<'_, Db>, id: i64) -> AppResult<()> {
    let item = find(&db, id)?;
    write_system_clipboard(&item.content)
}
