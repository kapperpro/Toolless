//! Notizblock: CRUD im lokalen Dateisystem, eine JSON-Datei je Notiz
//! unter `<Datenordner>/notes/<id>.json`.

use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::error::AppResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: String,
    pub title: String,
    pub body: String,
    /// Millisekunden seit 1970.
    pub updated_at: u64,
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub struct NoteStore {
    dir: PathBuf,
}

impl NoteStore {
    pub fn new(data_dir: &std::path::Path) -> AppResult<Self> {
        let dir = data_dir.join("notes");
        fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    fn file_of(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    /// Neue, leere Notiz (wird sofort gespeichert).
    pub fn create(&self) -> AppResult<Note> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let note = Note {
            id: format!("{nanos:x}"),
            title: String::new(),
            body: String::new(),
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
            // Beschädigte Dateien überspringen statt die ganze Liste scheitern zu lassen.
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            if let Ok(note) = serde_json::from_str::<Note>(&text) {
                notes.push(note);
            }
        }
        notes.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        Ok(notes)
    }

    pub fn save(&self, note: &Note) -> AppResult<()> {
        let json = serde_json::to_string_pretty(note)
            .map_err(|e| crate::error::AppError::other(format!("Notiz: {e}")))?;
        fs::write(self.file_of(&note.id), json)?;
        Ok(())
    }

    pub fn delete(&self, id: &str) -> AppResult<()> {
        match fs::remove_file(self.file_of(id)) {
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err.into()),
            _ => Ok(()),
        }
    }
}
