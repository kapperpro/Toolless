//! Notizblock: CRUD im lokalen Dateisystem, eine verschlüsselte JSON-Datei je Notiz
//! unter `<Datenordner>/notes/<id>.json`.

use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::{
    crypto,
    error::{AppError, AppResult},
    services::{
        note_blocks::{self, Block},
        pdf::{self, PdfBlock, PdfImage},
    },
};

/// Größte Kantenlänge, mit der Bilder ins PDF übernommen werden.
const PDF_IMAGE_MAX_SIDE: u32 = 1400;

/// Dateiformate für den Export einer Notiz.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportFormat {
    Text,
    Markdown,
    Pdf,
}

impl ExportFormat {
    pub const ALL: [ExportFormat; 3] = [
        ExportFormat::Text,
        ExportFormat::Markdown,
        ExportFormat::Pdf,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ExportFormat::Text => "Textdatei (.txt)",
            ExportFormat::Markdown => "Markdown (.md)",
            ExportFormat::Pdf => "PDF (.pdf)",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Text => "txt",
            ExportFormat::Markdown => "md",
            ExportFormat::Pdf => "pdf",
        }
    }
}

fn image_err(err: image::ImageError) -> AppError {
    AppError::other(format!("Bild: {err}"))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: String,
    pub title: String,
    /// Unterkategorie; leer = „Ohne Kategorie“.
    #[serde(default)]
    pub category: String,
    pub body: String,
    /// Dateinamen der angehängten Bilder (im Unterordner `images`).
    #[serde(default)]
    pub images: Vec<String>,
    /// Millisekunden seit 1970.
    pub updated_at: u64,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Klartext-JSON beginnt mit „{“ – verschlüsselte Dateien mit der Kennung aus `crypto`.
fn bytes_were_encrypted(path: &std::path::Path) -> bool {
    fs::read(path)
        .map(|raw| !raw.starts_with(b"{"))
        .unwrap_or(true)
}

pub struct NoteStore {
    dir: PathBuf,
}

impl NoteStore {
    pub fn new(data_dir: &std::path::Path) -> AppResult<Self> {
        let dir = data_dir.join("notes");
        fs::create_dir_all(dir.join("images"))?;
        Ok(Self { dir })
    }

    fn file_of(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    /// Neue Notiz in der angegebenen Kategorie (wird sofort gespeichert).
    pub fn create(&self, category: &str, title: &str, body: &str) -> AppResult<Note> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let note = Note {
            id: format!("{nanos:x}"),
            title: title.to_owned(),
            category: category.trim().to_owned(),
            body: body.to_owned(),
            images: Vec::new(),
            updated_at: now_ms(),
        };
        self.save(&note)?;
        Ok(note)
    }

    /// Alle Notizen, zuletzt geänderte zuerst.
    pub fn list(&self) -> AppResult<Vec<Note>> {
        let mut notes = Vec::new();
        for entry in fs::read_dir(&self.dir)?.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            // Beschädigte oder fremde Dateien überspringen statt die ganze Liste scheitern zu lassen.
            let Ok(bytes) = crypto::read_file(&path) else {
                continue;
            };
            let Ok(note) = serde_json::from_slice::<Note>(&bytes) else {
                continue;
            };
            // Notizen aus älteren Versionen liegen noch im Klartext vor → verschlüsselt neu schreiben.
            if !bytes_were_encrypted(&path) {
                let _ = self.save(&note);
            }
            notes.push(note);
        }
        notes.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(notes)
    }

    pub fn save(&self, note: &Note) -> AppResult<()> {
        let json = serde_json::to_vec_pretty(note)
            .map_err(|e| crate::error::AppError::other(format!("Notiz: {e}")))?;
        crypto::write_file(&self.file_of(&note.id), &json)
    }

    pub fn delete(&self, id: &str) -> AppResult<()> {
        match fs::remove_file(self.file_of(id)) {
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err.into()),
            _ => Ok(()),
        }
    }

    // ───────── Bilder ─────────

    fn image_path(&self, name: &str) -> PathBuf {
        self.dir.join("images").join(name)
    }

    /// Speichert ein Bild verschlüsselt (als PNG) und liefert seinen Dateinamen.
    pub fn add_image(&self, image: &image::RgbaImage) -> AppResult<String> {
        let mut png = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(image_err)?;
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let name = format!("{nanos:x}.png");
        crypto::write_file(&self.image_path(&name), &png)?;
        Ok(name)
    }

    /// Übernimmt eine Bilddatei (PNG, JPEG, BMP, GIF, WebP) als Anhang.
    pub fn add_image_file(&self, path: &Path) -> AppResult<String> {
        let bytes = fs::read(path)?;
        let image = image::load_from_memory(&bytes).map_err(image_err)?;
        self.add_image(&image.to_rgba8())
    }

    pub fn load_image(&self, name: &str) -> AppResult<image::RgbaImage> {
        let bytes = crypto::read_file(&self.image_path(name))?;
        Ok(image::load_from_memory(&bytes)
            .map_err(image_err)?
            .to_rgba8())
    }

    pub fn delete_image(&self, name: &str) {
        // Fehlt die Datei bereits, ist das Ziel ohnehin erreicht.
        let _ = fs::remove_file(self.image_path(name));
    }

    // ───────── Export ─────────

    /// Schreibt die Notiz unverschlüsselt in eine Datei des gewählten Formats.
    /// Bilder stehen an ihrer Stelle im Text: Markdown legt sie als PNG-Dateien
    /// daneben, PDF bettet sie ein, reiner Text vermerkt nur „[Bild]“.
    pub fn export(&self, note: &Note, path: &Path, format: ExportFormat) -> AppResult<()> {
        let title = note.title.trim();
        let blocks = note_blocks::parse(&note.body);
        match format {
            ExportFormat::Text => {
                let mut text = String::new();
                if !title.is_empty() {
                    text.push_str(&format!("{title}\n\n"));
                }
                let parts: Vec<String> = blocks
                    .iter()
                    .map(|block| match block {
                        Block::Text(text) => text.clone(),
                        Block::Image(_) => "[Bild]".to_owned(),
                    })
                    .collect();
                text.push_str(&parts.join("\n"));
                fs::write(path, text)?;
            }
            ExportFormat::Markdown => {
                let stem = path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .unwrap_or("notiz");
                let mut parts = Vec::new();
                let mut image_number = 0;
                for block in &blocks {
                    match block {
                        Block::Text(text) => parts.push(text.clone()),
                        Block::Image(name) => {
                            image_number += 1;
                            let file_name = format!("{stem}_bild{image_number}.png");
                            self.load_image(name)?
                                .save(path.with_file_name(&file_name))
                                .map_err(image_err)?;
                            parts.push(format!("![Bild {image_number}]({file_name})"));
                        }
                    }
                }
                let mut text = String::new();
                if !title.is_empty() {
                    text.push_str(&format!("# {title}\n\n"));
                }
                text.push_str(&parts.join("\n"));
                text.push('\n');
                fs::write(path, text)?;
            }
            ExportFormat::Pdf => {
                let mut pdf_blocks = Vec::new();
                for block in blocks {
                    match block {
                        Block::Text(text) => pdf_blocks.push(PdfBlock::Text(text)),
                        Block::Image(name) => {
                            let mut image =
                                image::DynamicImage::ImageRgba8(self.load_image(&name)?);
                            if image.width().max(image.height()) > PDF_IMAGE_MAX_SIDE {
                                image = image.thumbnail(PDF_IMAGE_MAX_SIDE, PDF_IMAGE_MAX_SIDE);
                            }
                            // JPEG kennt keine Transparenz → nach RGB wandeln.
                            let rgb = image::DynamicImage::ImageRgb8(image.to_rgb8());
                            let mut jpeg = Vec::new();
                            rgb.write_to(&mut Cursor::new(&mut jpeg), image::ImageFormat::Jpeg)
                                .map_err(image_err)?;
                            pdf_blocks.push(PdfBlock::Image(PdfImage {
                                width: rgb.width(),
                                height: rgb.height(),
                                jpeg,
                            }));
                        }
                    }
                }
                fs::write(path, pdf::build(title, &pdf_blocks))?;
            }
        }
        Ok(())
    }
}
