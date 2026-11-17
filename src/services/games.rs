//! Game Timer: erkennt laufende Spiele an ihrem Installationsort (Steam, Epic, Riot …)
//! oder an einer Liste eigener Programme und zählt die Spielzeit je Spiel.

use serde::{Deserialize, Serialize};

/// Ordner, unter denen Spiele-Launcher ihre Spiele ablegen (klein geschrieben).
/// Der Ordner direkt dahinter trägt den Namen des Spiels.
const LIBRARY_MARKERS: &[&str] = &[
    "\\steamapps\\common\\",
    "\\epic games\\",
    "\\riot games\\",
    "\\gog galaxy\\games\\",
    "\\gog games\\",
    "\\ubisoft game launchers\\games\\",
    "\\origin games\\",
    "\\ea games\\",
    "\\xboxgames\\",
    "\\rockstar games\\",
];

/// Ordner in den Bibliotheken, die keine Spiele sind.
const NOT_GAMES: &[&str] = &[
    "launcher",
    "riot client",
    "steamworks shared",
    "steam controller configs",
    "directxredist",
    "epic online services",
    "social club",
];

/// Hilfsprogramme, die neben Spielen laufen.
const HELPER_PARTS: &[&str] = &[
    "crashhandler",
    "crashreport",
    "crashpad",
    "anticheat",
    "battleye",
    "beservice",
    "redist",
    "setup",
    "unins",
    "webhelper",
    "cefprocess",
    "overlay",
    "updater",
    "installer",
];

/// Hilfsprogramme, die nie als Spiel gelten (außer der Nutzer fügt sie selbst hinzu):
/// Übertaktungs-, Overlay- und Überwachungswerkzeuge.
const UTILITY_PARTS: &[&str] = &[
    "msiafterburner",
    "afterburner",
    "rtss",
    "encoderserver",
    "hwinfo",
    "cpuz",
    "gpuz",
    "hwmonitor",
    "nvcontainer",
    "nvidia share",
    "radeonsoftware",
    "amdrsserv",
    "obs64",
    "obs32",
    "streamlabs",
    "steamwebhelper",
    "toolless",
];

/// Spiele, die außerhalb der üblichen Bibliotheken liegen: Programmdatei → Name.
const KNOWN_GAMES: &[(&str, &str)] = &[
    ("minecraft.windows.exe", "Minecraft (Bedrock)"),
    ("robloxplayerbeta.exe", "Roblox"),
    ("osu!.exe", "osu!"),
    ("genshinimpact.exe", "Genshin Impact"),
    ("leagueclientux.exe", "League of Legends"),
];

/// Ordnernamen, an denen sich ein Minecraft erkennen lässt, das über Java läuft.
const MINECRAFT_PARTS: &[&str] = &[
    "minecraft",
    "modrinth",
    "prismlauncher",
    "curseforge",
    "gdlauncher",
    "atlauncher",
    "lunarclient",
    "multimc",
];

/// Vom Nutzer gepflegte Listen.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GameConfig {
    /// Eigene Programme: Anzeigename und Programmdatei (Pfad oder Dateiname).
    pub custom: Vec<(String, String)>,
    /// Namen, die nicht als Spiel gezählt werden sollen.
    pub ignored: Vec<String>,
    /// So viele Tage bleiben Sitzungen gespeichert (1–365; 0 = Standard).
    pub retention_days: u32,
    /// Höchstgröße der Datei in KB (0 = Standard).
    pub max_kb: u32,
}

pub const DEFAULT_RETENTION_DAYS: u32 = 30;
pub const DEFAULT_MAX_KB: u32 = 1024;

impl GameConfig {
    pub fn retention_secs(&self) -> u64 {
        let days = if self.retention_days == 0 {
            DEFAULT_RETENTION_DAYS
        } else {
            self.retention_days.clamp(1, 365)
        };
        days as u64 * 86_400
    }

    pub fn max_bytes(&self) -> usize {
        let kb = if self.max_kb == 0 {
            DEFAULT_MAX_KB
        } else {
            self.max_kb
        };
        kb as usize * 1024
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

fn stem(name: &str) -> &str {
    let lower = name.to_lowercase();
    if lower.ends_with(".exe") {
        &name[..name.len() - 4]
    } else {
        name
    }
}

/// Liefert den Namen des Spiels, wenn dieser Prozess eines ist.
pub fn detect(
    name: &str,
    exe: Option<&str>,
    has_window: bool,
    config: &GameConfig,
) -> Option<String> {
    let path = exe.unwrap_or(name).replace('/', "\\");
    let lower = path.to_lowercase();
    let lower_name = file_name(&lower).to_owned();

    // Eigene Programme gelten immer, auch ohne Fenster.
    for (title, target) in &config.custom {
        let target = target.replace('/', "\\").to_lowercase();
        if !target.is_empty() && (lower == target || lower_name == file_name(&target)) {
            return Some(title.clone());
        }
    }
    if !has_window {
        return None;
    }
    if UTILITY_PARTS.iter().any(|part| lower_name.contains(part)) {
        return None;
    }

    let found = KNOWN_GAMES
        .iter()
        .find(|(file, _)| lower_name == *file)
        .map(|(_, title)| (*title).to_owned())
        .or_else(|| {
            let java = lower_name == "javaw.exe" || lower_name == "java.exe";
            (java && MINECRAFT_PARTS.iter().any(|part| lower.contains(part)))
                .then(|| "Minecraft".to_owned())
        })
        .or_else(|| {
            if HELPER_PARTS.iter().any(|part| lower_name.contains(part)) {
                return None;
            }
            LIBRARY_MARKERS.iter().find_map(|marker| {
                let start = lower.find(marker)? + marker.len();
                // Gleiche Länge wie im Original: nur ASCII wird klein geschrieben.
                let folder = path.get(start..)?.split('\\').next()?;
                // Die Programmdatei muss in diesem Ordner (oder tiefer) liegen.
                let is_folder = path.len() > start + folder.len();
                (is_folder
                    && !folder.is_empty()
                    && !NOT_GAMES.contains(&folder.to_lowercase().as_str()))
                .then(|| folder.to_owned())
            })
        })?;

    let ignored = config
        .ignored
        .iter()
        .any(|entry| entry.eq_ignore_ascii_case(&found));
    (!ignored).then_some(found)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    /// Beginn in Sekunden seit 1970.
    pub start: u64,
    pub secs: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Game {
    pub name: String,
    /// Zuletzt gesehene Programmdatei (für das Icon).
    #[serde(default)]
    pub exe: String,
    pub total_secs: u64,
    #[serde(default)]
    pub sessions: Vec<Session>,
}

impl Game {
    pub fn last_played(&self) -> u64 {
        self.sessions
            .last()
            .map_or(0, |session| session.start + session.secs)
    }

    /// Spielzeit seit `since` (Sitzungen werden am Stichzeitpunkt abgeschnitten).
    pub fn secs_between(&self, since: u64, until: u64) -> u64 {
        self.sessions
            .iter()
            .map(|session| {
                let start = session.start.max(since);
                let end = (session.start + session.secs).min(until);
                end.saturating_sub(start)
            })
            .sum()
    }
}

/// Liegt das Ende der letzten Sitzung höchstens so lange zurück, läuft sie weiter.
const SESSION_GAP_SECS: u64 = 120;
/// So viele Sitzungen bleiben je Spiel gespeichert.
const MAX_SESSIONS: usize = 2000;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Library {
    pub games: Vec<Game>,
}

impl Library {
    /// Zählt `elapsed` Sekunden für das laufende Spiel.
    pub fn record(&mut self, name: &str, exe: &str, now: u64, elapsed: u64) {
        let index = match self.games.iter().position(|game| game.name == name) {
            Some(index) => index,
            None => {
                self.games.push(Game {
                    name: name.to_owned(),
                    exe: String::new(),
                    total_secs: 0,
                    sessions: Vec::new(),
                });
                self.games.len() - 1
            }
        };
        let game = &mut self.games[index];
        if !exe.is_empty() {
            game.exe = exe.to_owned();
        }
        game.total_secs += elapsed;
        match game.sessions.last_mut() {
            Some(last) if now.saturating_sub(last.start + last.secs) <= SESSION_GAP_SECS => {
                last.secs = now.saturating_sub(last.start).max(last.secs + elapsed);
            }
            _ => game.sessions.push(Session {
                start: now.saturating_sub(elapsed),
                secs: elapsed,
            }),
        }
        if game.sessions.len() > MAX_SESSIONS {
            let extra = game.sessions.len() - MAX_SESSIONS;
            game.sessions.drain(..extra);
        }
    }

    /// Löscht Sitzungen, die älter als `retention_secs` sind, und danach die ältesten,
    /// bis `size_of` höchstens `max_bytes` meldet. Die Gesamtzeit wird aus den
    /// verbleibenden Sitzungen neu berechnet; Spiele ohne Sitzung verschwinden.
    pub fn prune(
        &mut self,
        now: u64,
        retention_secs: u64,
        max_bytes: usize,
        size_of: impl Fn(&Library) -> usize,
    ) {
        let cutoff = now.saturating_sub(retention_secs);
        let mut changed = false;
        for game in &mut self.games {
            let before = game.sessions.len();
            game.sessions
                .retain(|session| session.start + session.secs >= cutoff);
            changed |= game.sessions.len() != before;
        }
        // Zu große Datei: die jeweils ältesten Sitzungen entfernen.
        let mut guard = 0;
        while size_of(self) > max_bytes && guard < 10_000 {
            guard += 1;
            let oldest = self
                .games
                .iter()
                .enumerate()
                .filter_map(|(index, game)| Some((index, game.sessions.first()?.start)))
                .min_by_key(|(_, start)| *start);
            let Some((index, _)) = oldest else { break };
            self.games[index].sessions.remove(0);
            changed = true;
        }
        if changed {
            for game in &mut self.games {
                game.total_secs = game.sessions.iter().map(|session| session.secs).sum();
            }
            self.games.retain(|game| !game.sessions.is_empty());
        }
    }

    pub fn remove(&mut self, name: &str) {
        self.games.retain(|game| game.name != name);
    }
}

fn csv_field(text: &str) -> String {
    if text.contains([';', '"', '\n']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_owned()
    }
}

/// Eine Zeile je Sitzung, mit Semikolon getrennt (öffnet sich direkt in Excel).
/// `format_time` wandelt einen Zeitstempel in lesbaren Text.
pub fn export_csv(library: &Library, format_time: impl Fn(u64) -> String) -> String {
    let mut out = String::from("Spiel;Beginn;Ende;Dauer (Minuten);Gesamt (Stunden)\n");
    for game in &library.games {
        let total_hours = format!("{:.2}", game.total_secs as f64 / 3600.0);
        for session in &game.sessions {
            out.push_str(&format!(
                "{};{};{};{:.1};{}\n",
                csv_field(&game.name),
                format_time(session.start),
                format_time(session.start + session.secs),
                session.secs as f64 / 60.0,
                total_hours,
            ));
        }
    }
    out
}

/// Programmname ohne Endung – als Vorschlag für eigene Einträge.
pub fn suggested_name(exe: &str) -> String {
    stem(file_name(exe)).to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_library_games() {
        let config = GameConfig::default();
        let steam = r"D:\SteamLibrary\steamapps\common\Hades II\Hades2.exe";
        assert_eq!(
            detect("Hades2.exe", Some(steam), true, &config).as_deref(),
            Some("Hades II")
        );
        assert_eq!(detect("Hades2.exe", Some(steam), false, &config), None);
        let epic = r"C:\Program Files\Epic Games\Fortnite\FortniteGame\Binaries\Win64\FortniteClient-Win64-Shipping.exe";
        assert_eq!(
            detect("x", Some(epic), true, &config).as_deref(),
            Some("Fortnite")
        );
        let launcher = r"C:\Program Files (x86)\Epic Games\Launcher\Portal\Binaries\Win64\EpicGamesLauncher.exe";
        assert_eq!(detect("x", Some(launcher), true, &config), None);
        let helper = r"D:\SteamLibrary\steamapps\common\Hades II\UnityCrashHandler64.exe";
        assert_eq!(detect("x", Some(helper), true, &config), None);
        assert_eq!(
            detect(
                "chrome.exe",
                Some(r"C:\Program Files\Google\Chrome\chrome.exe"),
                true,
                &config
            ),
            None
        );
        assert_eq!(
            detect(
                "steam.exe",
                Some(r"C:\Program Files (x86)\Steam\steam.exe"),
                true,
                &config
            ),
            None
        );
    }

    #[test]
    fn detects_minecraft_known_and_custom() {
        let mut config = GameConfig::default();
        let java =
            r"C:\Users\x\AppData\Roaming\ModrinthApp\meta\java_versions\zulu21\bin\javaw.exe";
        assert_eq!(
            detect("javaw.exe", Some(java), true, &config).as_deref(),
            Some("Minecraft")
        );
        assert_eq!(
            detect(
                "javaw.exe",
                Some(r"C:\Program Files\Java\bin\javaw.exe"),
                true,
                &config
            ),
            None
        );
        assert_eq!(
            detect("RobloxPlayerBeta.exe", None, true, &config).as_deref(),
            Some("Roblox")
        );

        config
            .custom
            .push(("Mein Tool".to_owned(), r"C:\Tools\tool.exe".to_owned()));
        assert_eq!(
            detect("tool.exe", Some(r"C:\Tools\tool.exe"), false, &config).as_deref(),
            Some("Mein Tool")
        );
        assert_eq!(
            detect("tool.exe", None, false, &config).as_deref(),
            Some("Mein Tool")
        );

        config.ignored.push("minecraft".to_owned());
        assert_eq!(detect("javaw.exe", Some(java), true, &config), None);
    }

    #[test]
    fn prunes_old_sessions_and_large_files() {
        let mut library = Library::default();
        library.record("A", "", 10_000_000, 100);
        library.record("A", "", 10_500_000, 100);
        library.record("B", "", 10_000_000 + 100, 50);
        // Aufbewahrung 100 000 s: nur Sitzungen ab Sekunde 10 400 000 bleiben (Ende zählt).
        library.prune(10_500_100, 100_000, usize::MAX, |_| 0);
        assert_eq!(library.games.len(), 1);
        assert_eq!(library.games[0].total_secs, 100);
        // Größenlimit: wirft die älteste Sitzung zuerst hinaus.
        let mut library = Library::default();
        library.record("A", "", 1000, 10);
        library.record("A", "", 5000, 10);
        library.record("A", "", 9000, 10);
        library.prune(9100, u64::MAX / 2, 2, |lib| {
            lib.games.iter().map(|game| game.sessions.len()).sum()
        });
        assert_eq!(library.games[0].sessions.len(), 2);
        assert_eq!(library.games[0].total_secs, 20);
    }

    #[test]
    fn records_sessions() {
        let mut library = Library::default();
        library.record("Spiel", "a.exe", 1_000, 2);
        library.record("Spiel", "a.exe", 1_002, 2);
        library.record("Spiel", "a.exe", 1_010, 8);
        assert_eq!(
            library.games[0].sessions,
            [Session {
                start: 998,
                secs: 12
            }]
        );
        assert_eq!(library.games[0].total_secs, 12);
        // Nach einer langen Pause beginnt eine neue Sitzung.
        library.record("Spiel", "", 5_000, 10);
        assert_eq!(library.games[0].sessions.len(), 2);
        assert_eq!(library.games[0].exe, "a.exe");
        assert_eq!(library.games[0].secs_between(1_004, 4_995), 6 + 5);
        assert_eq!(library.games[0].last_played(), 5_000);

        let csv = export_csv(&library, |time| time.to_string());
        assert!(csv
            .lines()
            .nth(1)
            .unwrap()
            .starts_with("Spiel;998;1010;0.2;"));
        assert_eq!(csv.lines().count(), 3);
        assert_eq!(suggested_name(r"C:\x\Game.EXE"), "Game");
    }
}
