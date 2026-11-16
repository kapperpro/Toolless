//! Erweiterter Clipboard-Manager: System-Zwischenablage (arboard) + gespeicherte Einträge.

use std::{
    borrow::Cow,
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use arboard::Clipboard;
use serde::{Deserialize, Serialize};

use crate::{
    crypto,
    db::Db,
    error::{AppError, AppResult},
};

/// Unterordner des Datenordners für gespeicherte Bilder.
const IMAGE_DIR: &str = "clips";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipItem {
    pub id: i64,
    pub title: String,
    pub category: String,
    pub content: String,
    /// Dateiname eines gespeicherten Bildes (statt Text), siehe [`save_image`].
    #[serde(default)]
    pub image: Option<String>,
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

/// Liest ein Bild aus der System-Zwischenablage (`None`, wenn dort keines liegt).
pub fn read_system_image() -> AppResult<Option<image::RgbaImage>> {
    match Clipboard::new()?.get_image() {
        Ok(data) => Ok(image::RgbaImage::from_raw(
            data.width as u32,
            data.height as u32,
            data.bytes.into_owned(),
        )),
        Err(arboard::Error::ContentNotAvailable) => Ok(None),
        Err(err) => Err(err.into()),
    }
}

/// Schreibt Text in die System-Zwischenablage.
pub fn write_system(text: &str) -> AppResult<()> {
    Clipboard::new()?.set_text(text.to_owned())?;
    Ok(())
}

pub fn list(db: &Db) -> AppResult<Vec<ClipItem>> {
    db.read(|data| data.clips.clone())
}

pub fn save(db: &Db, title: &str, category: &str, content: &str) -> AppResult<ClipItem> {
    if content.trim().is_empty() {
        return Err(AppError::other("Der Eintrag enthält keinen Text."));
    }
    let title = normalize_title(title, content);
    let category = category.trim().to_owned();
    db.write(|data| {
        data.next_clip_id += 1;
        let item = ClipItem {
            id: data.next_clip_id,
            title,
            category,
            content: content.to_owned(),
            image: None,
        };
        data.clips.insert(0, item.clone());
        item
    })
}

pub fn update(db: &Db, item: &ClipItem) -> AppResult<ClipItem> {
    let updated = ClipItem {
        id: item.id,
        title: normalize_title(&item.title, &item.content),
        category: item.category.trim().to_owned(),
        content: item.content.clone(),
        image: item.image.clone(),
    };
    let found =
        db.write(
            |data| match data.clips.iter_mut().find(|clip| clip.id == updated.id) {
                Some(slot) => {
                    *slot = updated.clone();
                    true
                }
                None => false,
            },
        )?;
    if found {
        Ok(updated)
    } else {
        Err(AppError::other("Eintrag nicht gefunden."))
    }
}

pub fn delete(db: &Db, id: i64) -> AppResult<()> {
    db.write(|data| data.clips.retain(|clip| clip.id != id))
}

// ───────── Bilder ─────────

fn image_err(err: image::ImageError) -> AppError {
    AppError::other(format!("Bild: {err}"))
}

fn image_path(data_dir: &Path, name: &str) -> PathBuf {
    data_dir.join(IMAGE_DIR).join(name)
}

/// Speichert ein Bild (verschlüsselt, als PNG) als neuen Eintrag.
pub fn save_image(
    db: &Db,
    data_dir: &Path,
    title: &str,
    category: &str,
    image: &image::RgbaImage,
) -> AppResult<ClipItem> {
    let mut png = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(image_err)?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let name = format!("{nanos:x}.png");
    fs::create_dir_all(data_dir.join(IMAGE_DIR))?;
    crypto::write_file(&image_path(data_dir, &name), &png)?;

    let title = if title.trim().is_empty() {
        format!("Bild {}×{}", image.width(), image.height())
    } else {
        title.trim().to_owned()
    };
    let category = category.trim().to_owned();
    db.write(|data| {
        data.next_clip_id += 1;
        let item = ClipItem {
            id: data.next_clip_id,
            title,
            category,
            content: String::new(),
            image: Some(name),
        };
        data.clips.insert(0, item.clone());
        item
    })
}

pub fn load_image(data_dir: &Path, name: &str) -> AppResult<image::RgbaImage> {
    let bytes = crypto::read_file(&image_path(data_dir, name))?;
    Ok(image::load_from_memory(&bytes)
        .map_err(image_err)?
        .to_rgba8())
}

pub fn delete_image(data_dir: &Path, name: &str) {
    // Fehlt die Datei bereits, ist das Ziel ohnehin erreicht.
    let _ = fs::remove_file(image_path(data_dir, name));
}

/// Legt ein Bild in die System-Zwischenablage.
pub fn write_system_image(image: &image::RgbaImage) -> AppResult<()> {
    Clipboard::new()?.set_image(arboard::ImageData {
        width: image.width() as usize,
        height: image.height() as usize,
        bytes: Cow::Borrowed(image.as_raw()),
    })?;
    Ok(())
}
