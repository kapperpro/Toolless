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
    /// Ordner im Startmenü (oberste Ebene); leer = direkt unter „Programme“.
    pub folder: String,
    /// Systemwerkzeug, Dokumentation o. Ä. – standardmäßig ausgeblendet.
    pub system: bool,
}

/// Startmenü-Ordner, die fast nur Systemwerkzeuge enthalten.
const SYSTEM_FOLDERS: &[&str] = &[
    "administrative tools",
    "windows tools",
    "windows system",
    "system tools",
    "accessibility",
    "windows ease of access",
    "windows powershell",
    "windows kits",
    "microsoft office tools",
    "microsoft office-tools",
    "visual studio tools",
    "maintenance",
    "startup",
    "autostart",
    "/system/applications/utilities",
];

/// Namensbestandteile von Einträgen, die keine eigentlichen Programme sind
/// (Hilfen, Webseiten-Links, Beispiele, Diagnosewerkzeuge …).
const NOISE_WORDS: &[&str] = &[
    "uninstall",
    "deinstall",
    "readme",
    "read me",
    "release notes",
    "changelog",
    "what's new",
    "help",
    "hilfe",
    "documentation",
    "dokumentation",
    "manual",
    "handbuch",
    "reference",
    "website",
    "web site",
    "homepage",
    "license",
    "lizenz",
    "faq",
    "sample",
    "example",
    "verifier",
    "character map",
    "zeichentabelle",
    "odbc",
    "iscsi",
    "memory diagnostic",
    "recovery drive",
    "steps recorder",
    "error reporting",
    "telemetry",
    "debugg",
    "command prompt for",
    "command prompt",
    "eingabeaufforderung",
    "developer powershell",
    "native tools",
    "cross tools",
    "spreadsheet compare",
    "database compare",
    "compare",
    "language preferences",
    "spracheinstellungen",
    "upload center",
    "recording manager",
    "office tools",
    "dashboard for office",
    "config",
    "konfig",
    "setup",
    "installer",
    "updater",
    "update",
    "repair",
    "reparatur",
    "troubleshoot",
    "diagnos",
    "report",
    "feedback",
    "support",
    "register",
    "activation",
    "aktivierung",
    "about ",
    "über ",
    "visit ",
    "online",
    "on the web",
    "user guide",
    "quick start",
    "getting started",
    "erste schritte",
    "module docs",
    "localization",
    "skin format",
    "safe mode",
    "abgesicherter modus",
    "resource monitor",
    "registry editor",
    "event viewer",
    "task scheduler",
    "system configuration",
    "system information",
    "disk cleanup",
    "defragment",
    "component services",
    "computer management",
    "performance monitor",
    "print management",
    "security policy",
    "security configuration",
    "services",
    "remote desktop",
    "quick assist",
    "magnifier",
    "narrator",
    "on-screen keyboard",
    "speech recognition",
    "math input",
    "wordpad",
    "xps viewer",
    "windows fax",
    "internet explorer",
    "run",
];

fn is_system_entry(path: &Path, name: &str) -> bool {
    let path = path.to_string_lossy().to_lowercase().replace('\\', "/");
    let name = name.to_lowercase();
    let in_system_folder = SYSTEM_FOLDERS
        .iter()
        .any(|folder| path.contains(&format!("/{}/", folder.trim_matches('/'))));
    // Kurze Begriffe („run“, „help“) nur als ganzes Wort werten, damit z. B.
    // „Runescape“ nicht ausgeblendet wird.
    let words: Vec<&str> = name.split(|c: char| !c.is_alphanumeric()).collect();
    let noisy = NOISE_WORDS.iter().any(|noise| {
        if noise.len() <= 6 && !noise.contains(' ') {
            words.contains(noise)
        } else {
            name.contains(noise)
        }
    });
    in_system_folder || noisy
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

/// Oberster Ordner eines Eintrags relativ zum App-Verzeichnis (wie im Startmenü).
fn top_folder(root: &Path, path: &Path) -> String {
    path.parent()
        .and_then(|parent| parent.strip_prefix(root).ok())
        .and_then(|relative| relative.components().next())
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Listet alle installierten Programme aus den Standard-Verzeichnissen.
/// Schnell: Es werden nur Verzeichnisse gelesen – Icons lädt die Oberfläche
/// anschließend getrennt über `services::icons`.
pub fn scan() -> Vec<AppEntry> {
    // Nach Name deduplizieren (z. B. gleiche Verknüpfung für „Alle Benutzer“ und Nutzer)
    // und alphabetisch ohne Beachtung der Groß-/Kleinschreibung sortieren.
    let mut by_name = BTreeMap::new();
    for root in app_roots() {
        let mut found = Vec::new();
        collect(&root, 0, &mut found);
        for path in found {
            if let (Some(name), Some(path_str)) = (display_name(&path), path.to_str()) {
                by_name
                    .entry(name.to_lowercase())
                    .or_insert_with(|| AppEntry {
                        system: is_system_entry(&path, &name),
                        folder: top_folder(&root, &path),
                        path: path_str.to_owned(),
                        name,
                    });
            }
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

/// Öffnet eine beliebige Datei mit dem dafür zuständigen Programm – so starten auch
/// Skripte (.bat, .cmd, .ps1), Verknüpfungen oder Dokumente.
fn open_file(path: &Path) -> AppResult<()> {
    if !path.exists() {
        return Err(AppError::other(format!(
            "Datei nicht gefunden: {}",
            path.display()
        )));
    }
    #[cfg(target_os = "windows")]
    let program = "explorer";
    #[cfg(target_os = "macos")]
    let program = "open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let program = "xdg-open";

    Command::new(program).arg(path).spawn()?;
    Ok(())
}

/// Startet mehrere Apps; liefert (Anzahl gestartet, erste Fehlermeldung).
/// `custom` sind die vom Nutzer selbst hinzugefügten Einträge: Nur sie dürfen
/// außerhalb der App-Verzeichnisse liegen.
pub fn launch(paths: &[String], custom: &[String]) -> (usize, Option<String>) {
    let mut started = 0;
    let mut first_error = None;
    for path in paths {
        let result = if custom.contains(path) {
            open_file(Path::new(path))
        } else {
            validate(Path::new(path)).and_then(|p| spawn(&p))
        };
        match result {
            Ok(()) => started += 1,
            Err(err) => {
                first_error.get_or_insert(err.to_string());
            }
        }
    }
    (started, first_error)
}
