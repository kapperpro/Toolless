//! Multi App Starter: scannt die Standard-App-Verzeichnisse des Betriebssystems
//! und startet ausgewählte Programme über Systembefehle.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::error::{AppError, AppResult};

/// Maximale Verzeichnistiefe beim Scannen (Startmenü-Unterordner etc.).
const MAX_DEPTH: usize = 4;

#[derive(Debug, Clone)]
pub struct AppEntry {
    pub name: String,
    pub path: String,
}

/// Dateiendung, an der eine startbare App auf diesem System erkannt wird.
#[cfg(target_os = "windows")]
const APP_EXTENSION: &str = "lnk";
#[cfg(target_os = "macos")]
const APP_EXTENSION: &str = "app";
#[cfg(all(unix, not(target_os = "macos")))]
const APP_EXTENSION: &str = "desktop";

/// Standard-App-Verzeichnisse des jeweiligen Betriebssystems.
fn app_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    #[cfg(target_os = "windows")]
    for var in ["PROGRAMDATA", "APPDATA"] {
        if let Some(base) = std::env::var_os(var) {
            roots.push(PathBuf::from(base).join(r"Microsoft\Windows\Start Menu\Programs"));
        }
    }

    #[cfg(target_os = "macos")]
    {
        roots.push(PathBuf::from("/Applications"));
        roots.push(PathBuf::from("/System/Applications"));
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(home).join("Applications"));
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        roots.push(PathBuf::from("/usr/share/applications"));
        roots.push(PathBuf::from("/usr/local/share/applications"));
        roots.push(PathBuf::from("/var/lib/flatpak/exports/share/applications"));
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(home).join(".local/share/applications"));
        }
    }

    roots
}

fn is_app(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case(APP_EXTENSION))
}

/// Anzeigename einer App; `None` blendet den Eintrag aus.
#[cfg(all(unix, not(target_os = "macos")))]
fn display_name(path: &Path) -> Option<String> {
    // .desktop-Datei: `Name=` lesen, versteckte Einträge überspringen.
    let content = fs::read_to_string(path).ok()?;
    let mut name = None;
    for line in content.lines() {
        let line = line.trim();
        if line == "NoDisplay=true" || line == "Hidden=true" {
            return None;
        }
        if name.is_none() {
            name = line.strip_prefix("Name=").map(str::to_owned);
        }
    }
    name
}

/// Anzeigename einer App; `None` blendet den Eintrag aus.
#[cfg(not(all(unix, not(target_os = "macos"))))]
fn display_name(path: &Path) -> Option<String> {
    let name = path.file_stem()?.to_str()?.to_owned();
    let lower = name.to_lowercase();
    // Deinstallations-Verknüpfungen gehören nicht in einen App-Starter.
    if lower.contains("uninstall") || lower.contains("deinstall") {
        return None;
    }
    Some(name)
}

fn collect(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if is_app(&path) {
            // Auf macOS sind .app-Bundles Verzeichnisse – nicht hineinlaufen.
            out.push(path);
        } else if depth < MAX_DEPTH && path.is_dir() {
            collect(&path, depth + 1, out);
        }
    }
}

/// Listet alle installierten Programme aus den Standard-Verzeichnissen.
pub fn scan() -> Vec<AppEntry> {
    let mut found = Vec::new();
    for root in app_roots() {
        collect(&root, 0, &mut found);
    }

    // Nach Name deduplizieren (z. B. gleiche Verknüpfung für „Alle Benutzer“ und Nutzer)
    // und alphabetisch ohne Beachtung der Groß-/Kleinschreibung sortieren.
    let mut by_name = BTreeMap::new();
    for path in found {
        if let (Some(name), Some(path_str)) = (display_name(&path), path.to_str()) {
            by_name.entry(name.to_lowercase()).or_insert(AppEntry {
                name,
                path: path_str.to_owned(),
            });
        }
    }
    by_name.into_values().collect()
}

/// Nur Einträge aus den gescannten App-Verzeichnissen dürfen gestartet werden –
/// das Frontend kann so keine beliebigen Dateien ausführen lassen.
fn validate(path: &Path) -> AppResult<PathBuf> {
    if !is_app(path) {
        return Err(AppError::other("Kein startbarer App-Eintrag"));
    }
    let canonical = fs::canonicalize(path)?;
    let allowed = app_roots()
        .iter()
        .filter_map(|root| fs::canonicalize(root).ok())
        .any(|root| canonical.starts_with(root));
    if !allowed {
        return Err(AppError::other(
            "Pfad liegt außerhalb der App-Verzeichnisse",
        ));
    }
    Ok(path.to_path_buf())
}

fn spawn(path: &Path) -> AppResult<()> {
    #[cfg(target_os = "windows")]
    let mut command = {
        // Der Explorer löst .lnk-Verknüpfungen inkl. Argumenten und Arbeitsverzeichnis auf.
        let mut command = Command::new("explorer");
        command.arg(path);
        command
    };

    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(path);
        command
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let id = path
            .file_stem()
            .ok_or_else(|| AppError::other("Ungültiger Desktop-Eintrag"))?;
        let mut command = Command::new("gtk-launch");
        command.arg(id);
        command
    };

    command.spawn()?;
    Ok(())
}

/// Startet mehrere Apps; liefert (Anzahl gestartet, erste Fehlermeldung).
pub fn launch(paths: &[String]) -> (usize, Option<String>) {
    let mut started = 0;
    let mut first_error = None;
    for path in paths {
        match validate(Path::new(path)).and_then(|p| spawn(&p)) {
            Ok(()) => started += 1,
            Err(err) => {
                first_error.get_or_insert(err.to_string());
            }
        }
    }
    (started, first_error)
}
