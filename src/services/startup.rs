//! Autostart-Programme von Windows – wie die Registerkarte „Autostart“ im Task-Manager.
//!
//! Gelesen werden die Registry-Schlüssel `Run` (Benutzer, alle Benutzer, 32-Bit) und der
//! Autostart-Ordner des Benutzers. Aktiviert oder deaktiviert wird – genau wie in Windows –
//! über den Schlüssel `StartupApproved`; der Eintrag selbst bleibt unverändert. Die
//! Registry wird über `reg.exe` gelesen und geschrieben.

use std::{collections::HashMap, path::PathBuf};

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_32: &str = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";

/// Woher ein Autostart-Eintrag stammt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    User,
    Machine,
    Machine32,
    Folder,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::User => "Benutzer",
            Source::Machine => "Alle Benutzer",
            Source::Machine32 => "Alle Benutzer (32-Bit)",
            Source::Folder => "Autostart-Ordner",
        }
    }

    /// Schlüssel, in dem Windows Aktiv/Inaktiv vermerkt.
    fn approved_key(self) -> String {
        match self {
            Source::User => format!(r"HKCU\{APPROVED}\Run"),
            Source::Machine => format!(r"HKLM\{APPROVED}\Run"),
            Source::Machine32 => format!(r"HKLM\{APPROVED}\Run32"),
            Source::Folder => format!(r"HKCU\{APPROVED}\StartupFolder"),
        }
    }

    fn run_key(self) -> Option<String> {
        match self {
            Source::User => Some(format!(r"HKCU\{RUN}")),
            Source::Machine => Some(format!(r"HKLM\{RUN}")),
            Source::Machine32 => Some(format!(r"HKLM\{RUN_32}")),
            Source::Folder => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Entry {
    /// Anzeigename.
    pub name: String,
    /// Name des Werts bzw. der Datei – dieser Schlüssel steht in `StartupApproved`.
    pub key: String,
    pub command: String,
    pub source: Source,
    pub enabled: bool,
}

/// Führt `reg.exe` ohne Konsolenfenster aus.
fn reg(args: &[&str]) -> Result<String, String> {
    use std::process::{Command, Stdio};
    let mut command = Command::new("reg");
    command.args(args).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let output = command.output().map_err(|err| err.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(if error.is_empty() {
            "Zugriff verweigert oder Eintrag nicht gefunden".to_owned()
        } else {
            error
        })
    }
}

/// Zerlegt die Ausgabe von `reg query` in (Name, Typ, Daten).
pub fn parse_reg_query(output: &str) -> Vec<(String, String, String)> {
    let mut values = Vec::new();
    for line in output.lines() {
        let Some(line) = line.strip_prefix("    ") else {
            continue;
        };
        let Some(split) = line.find("    REG_") else {
            continue;
        };
        let name = line[..split].trim().to_owned();
        let rest = &line[split + 4..];
        let (kind, data) = rest.split_once("    ").unwrap_or((rest, ""));
        if name.is_empty() {
            continue;
        }
        values.push((name, kind.trim().to_owned(), data.trim().to_owned()));
    }
    values
}

/// `020000000000000000000000` → aktiv. Eine ungerade erste Zahl heißt „deaktiviert“.
pub fn is_enabled(approval: &str) -> bool {
    let first = approval
        .get(..2)
        .and_then(|hex| u8::from_str_radix(hex, 16).ok());
    first.map_or(true, |byte| byte & 1 == 0)
}

/// Zustand aller Einträge eines `StartupApproved`-Schlüssels: Name → aktiv.
fn approvals(source: Source) -> HashMap<String, bool> {
    reg(&["query", &source.approved_key()])
        .map(|output| {
            parse_reg_query(&output)
                .into_iter()
                .map(|(name, _, data)| (name.to_lowercase(), is_enabled(&data)))
                .collect()
        })
        .unwrap_or_default()
}

fn startup_folder() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    Some(
        PathBuf::from(base)
            .join("Microsoft")
            .join("Windows")
            .join("Start Menu")
            .join("Programs")
            .join("Startup"),
    )
}

/// Alle Autostart-Einträge, alphabetisch.
pub fn list() -> Vec<Entry> {
    let mut entries: Vec<Entry> = Vec::new();
    for source in [Source::User, Source::Machine, Source::Machine32] {
        let Some(key) = source.run_key() else {
            continue;
        };
        let Ok(output) = reg(&["query", &key]) else {
            continue;
        };
        let approved = approvals(source);
        for (name, _, command) in parse_reg_query(&output) {
            let enabled = approved.get(&name.to_lowercase()).copied().unwrap_or(true);
            entries.push(Entry {
                key: name.clone(),
                name,
                command,
                source,
                enabled,
            });
        }
    }

    if let Some(folder) = startup_folder() {
        let approved = approvals(Source::Folder);
        if let Ok(read) = std::fs::read_dir(&folder) {
            for item in read.flatten() {
                let file = item.file_name().to_string_lossy().into_owned();
                if file.eq_ignore_ascii_case("desktop.ini") {
                    continue;
                }
                let name = item
                    .path()
                    .file_stem()
                    .map_or_else(|| file.clone(), |stem| stem.to_string_lossy().into_owned());
                let enabled = approved.get(&file.to_lowercase()).copied().unwrap_or(true);
                entries.push(Entry {
                    name,
                    command: item.path().to_string_lossy().into_owned(),
                    key: file,
                    source: Source::Folder,
                    enabled,
                });
            }
        }
    }
    entries.sort_by_key(|entry| entry.name.to_lowercase());
    entries
}

/// Schaltet einen Eintrag ein oder aus. Einträge für „alle Benutzer“ brauchen
/// Administratorrechte.
pub fn set_enabled(entry: &Entry, enabled: bool) -> Result<(), String> {
    let data = if enabled {
        "020000000000000000000000"
    } else {
        "030000000000000000000000"
    };
    reg(&[
        "add",
        &entry.source.approved_key(),
        "/v",
        &entry.key,
        "/t",
        "REG_BINARY",
        "/d",
        data,
        "/f",
    ])
    .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_reg_output() {
        let output = "\r\nHKEY_CURRENT_USER\\Software\\Microsoft\\Windows\\CurrentVersion\\Run\r\n    Discord    REG_SZ    \"C:\\Users\\a\\Update.exe\" --processStart Discord.exe\r\n    My App Name    REG_SZ    C:\\x y\\app.exe\r\n\r\n";
        let values = parse_reg_query(output);
        assert_eq!(values.len(), 2);
        assert_eq!(values[0].0, "Discord");
        assert_eq!(values[0].1, "REG_SZ");
        assert!(values[0].2.contains("--processStart"));
        assert_eq!(values[1].0, "My App Name");
        assert_eq!(values[1].2, "C:\\x y\\app.exe");
    }

    #[test]
    fn reads_approval_state() {
        assert!(is_enabled("020000000000000000000000"));
        assert!(is_enabled("060000000000000000000000"));
        assert!(!is_enabled("030000000000000000000000"));
        assert!(!is_enabled("070000000000000000000000"));
        assert!(is_enabled(""));
    }
}
