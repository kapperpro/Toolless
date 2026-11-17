//! Sicherung und Wiederherstellung: alle Daten von Toolless (Einstellungen, Zwischenablage,
//! Notizen samt Bildern, Verlauf, Spielzeiten) in einer einzigen Datei. Es gibt keine Konten –
//! so lässt sich alles auf einen anderen Rechner mitnehmen. Die Datei ist *nicht* verschlüsselt
//! (die Verschlüsselung der Platte ist an das Windows-Konto gebunden), also sicher aufbewahren.

use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    crypto,
    error::{AppError, AppResult},
};

const FORMAT: &str = "toolless-backup";

#[derive(Serialize, Deserialize)]
struct Entry {
    /// Pfad relativ zum Datenordner, mit `/` getrennt.
    path: String,
    /// Inhalt, Base64.
    data: String,
}

#[derive(Serialize, Deserialize)]
struct Backup {
    format: String,
    version: u32,
    files: Vec<Entry>,
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let mut buffer = 0u32;
    let mut bits = 0;
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => break,
            b'\n' | b'\r' | b' ' => continue,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Sammelt alle Dateien unterhalb von `dir` (ohne temporäre Dateien).
fn collect(dir: &Path, base: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, base, files);
        } else if path.extension().and_then(|e| e.to_str()) != Some("tmp") {
            if let Ok(relative) = path.strip_prefix(base) {
                files.push(relative.to_path_buf());
            }
        }
    }
}

/// Schreibt alle Daten nach `target`. Liefert die Zahl der gesicherten Dateien.
pub fn export(data_dir: &Path, target: &Path) -> AppResult<usize> {
    let mut paths = Vec::new();
    collect(data_dir, data_dir, &mut paths);
    paths.sort();
    let mut files = Vec::new();
    for relative in paths {
        // Entschlüsselt lesen, damit die Sicherung auch unter einem anderen Konto funktioniert.
        let Ok(bytes) = crypto::read_file(&data_dir.join(&relative)) else {
            continue;
        };
        let path = relative
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        files.push(Entry {
            path,
            data: encode(&bytes),
        });
    }
    let count = files.len();
    let json = serde_json::to_vec(&Backup {
        format: FORMAT.to_owned(),
        version: 1,
        files,
    })
    .map_err(|e| AppError::other(format!("Sicherung: {e}")))?;
    fs::write(target, json)?;
    Ok(count)
}

/// Nur harmlose relative Pfade zulassen.
fn safe_path(data_dir: &Path, path: &str) -> Option<PathBuf> {
    if path.is_empty() || path.contains('\\') || path.contains(':') {
        return None;
    }
    let relative = Path::new(path);
    if !relative
        .components()
        .all(|part| matches!(part, Component::Normal(_)))
    {
        return None;
    }
    Some(data_dir.join(relative))
}

/// Stellt eine Sicherung wieder her und überschreibt die vorhandenen Daten.
/// Liefert die Zahl der wiederhergestellten Dateien.
pub fn import(data_dir: &Path, source: &Path) -> AppResult<usize> {
    let bytes = fs::read(source)?;
    let backup: Backup = serde_json::from_slice(&bytes)
        .map_err(|_| AppError::other("Das ist keine Toolless-Sicherung."))?;
    if backup.format != FORMAT {
        return Err(AppError::other("Das ist keine Toolless-Sicherung."));
    }
    // Erst alles prüfen und dekodieren, dann schreiben – so bleibt bei einem Fehler alles heil.
    let mut prepared = Vec::new();
    for entry in &backup.files {
        let path = safe_path(data_dir, &entry.path)
            .ok_or_else(|| AppError::other("Die Sicherung enthält ungültige Pfade."))?;
        let data =
            decode(&entry.data).ok_or_else(|| AppError::other("Die Sicherung ist beschädigt."))?;
        prepared.push((path, data));
    }
    for (path, data) in &prepared {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        crypto::write_file(path, data)?;
    }
    Ok(prepared.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_roundtrip() {
        for len in 0..40usize {
            let data: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            assert_eq!(decode(&encode(&data)).as_deref(), Some(data.as_slice()));
        }
        assert_eq!(encode(b"Man"), "TWFu");
        assert_eq!(encode(b"Ma"), "TWE=");
        assert_eq!(encode(b"M"), "TQ==");
    }

    #[test]
    fn rejects_unsafe_paths() {
        let base = Path::new("/data");
        assert!(safe_path(base, "notes/a.json").is_some());
        assert!(safe_path(base, "../x").is_none());
        assert!(safe_path(base, "/etc/passwd").is_none());
        assert!(safe_path(base, "C:/x").is_none());
        assert!(safe_path(base, "a\\b").is_none());
        assert!(safe_path(base, "").is_none());
    }
}
