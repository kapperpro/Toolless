//! Datenspeicher der App: gespeicherte Zwischenablage-Einträge und Einstellungen.
//!
//! Alles liegt in einer einzigen JSON-Datei, die verschlüsselt auf die Platte
//! geschrieben wird (siehe `crate::crypto`). Die Datenmenge ist klein, deshalb
//! wird die Datei bei jeder Änderung komplett neu geschrieben.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Serialize};

use crate::{
    crypto,
    error::{AppError, AppResult},
    services::clipboard::ClipItem,
};

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Data {
    pub next_clip_id: i64,
    /// Neueste zuerst.
    pub clips: Vec<ClipItem>,
    pub settings: BTreeMap<String, String>,
}

/// Thread-sicherer Zugriff auf den Datenspeicher.
pub struct Db {
    path: PathBuf,
    data: Mutex<Data>,
}

impl Db {
    pub fn open(path: &Path) -> AppResult<Self> {
        let data = if path.exists() {
            let bytes = crypto::read_file(path)?;
            serde_json::from_slice(&bytes)
                .map_err(|e| AppError::other(format!("Datenspeicher ist beschädigt: {e}")))?
        } else {
            Data::default()
        };
        Ok(Self {
            path: path.to_path_buf(),
            data: Mutex::new(data),
        })
    }

    /// Liest aus dem Speicher.
    pub fn read<T>(&self, f: impl FnOnce(&Data) -> T) -> AppResult<T> {
        let data = self
            .data
            .lock()
            .map_err(|_| AppError::other("Datenspeicher ist nicht verfügbar"))?;
        Ok(f(&data))
    }

    /// Ändert den Speicher und schreibt ihn anschließend verschlüsselt auf die Platte.
    pub fn write<T>(&self, f: impl FnOnce(&mut Data) -> T) -> AppResult<T> {
        let mut data = self
            .data
            .lock()
            .map_err(|_| AppError::other("Datenspeicher ist nicht verfügbar"))?;
        let result = f(&mut data);
        let json = serde_json::to_vec(&*data)
            .map_err(|e| AppError::other(format!("Datenspeicher: {e}")))?;
        crypto::write_file(&self.path, &json)?;
        Ok(result)
    }

    pub fn get_setting(&self, key: &str) -> AppResult<Option<String>> {
        self.read(|data| data.settings.get(key).cloned())
    }

    pub fn set_setting(&self, key: &str, value: &str) -> AppResult<()> {
        self.write(|data| {
            data.settings.insert(key.to_owned(), value.to_owned());
        })
    }
}
