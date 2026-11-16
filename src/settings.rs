//! Benutzereinstellungen (Design, Verhalten) – als JSON im verschlüsselten Datenspeicher.

use eframe::egui::Color32;
use serde::{Deserialize, Serialize};

use crate::{
    db::Db,
    error::{AppError, AppResult},
    services::monitor::PowerConfig,
};

const SETTING_KEY: &str = "settings";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub const ALL: [Appearance; 3] = [Appearance::System, Appearance::Light, Appearance::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Appearance::System => "Automatisch",
            Appearance::Light => "Hell",
            Appearance::Dark => "Dunkel",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Accent {
    #[default]
    Blue,
    Purple,
    Pink,
    Red,
    Orange,
    Green,
    Graphite,
    Teal,
    Yellow,
}

impl Accent {
    pub const ALL: [Accent; 9] = [
        Accent::Blue,
        Accent::Purple,
        Accent::Pink,
        Accent::Red,
        Accent::Orange,
        Accent::Yellow,
        Accent::Green,
        Accent::Teal,
        Accent::Graphite,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Accent::Blue => "Blau",
            Accent::Purple => "Lila",
            Accent::Pink => "Pink",
            Accent::Red => "Rot",
            Accent::Orange => "Orange",
            Accent::Green => "Grün",
            Accent::Graphite => "Graphit",
            Accent::Teal => "Türkis",
            Accent::Yellow => "Gelb",
        }
    }

    /// Akzentfarbe; Blau entspricht dem Windows-Standard (hell/dunkel leicht verschieden).
    pub fn color(self, dark: bool) -> Color32 {
        let (light, night) = match self {
            Accent::Blue => ((0, 103, 192), (96, 205, 255)),
            Accent::Teal => ((0, 135, 125), (70, 205, 190)),
            Accent::Yellow => ((190, 140, 0), (255, 200, 40)),
            Accent::Purple => ((165, 78, 210), (191, 90, 242)),
            Accent::Pink => ((240, 50, 100), (255, 55, 95)),
            Accent::Red => ((235, 55, 45), (255, 69, 58)),
            Accent::Orange => ((240, 135, 0), (255, 159, 10)),
            Accent::Green => ((36, 170, 60), (50, 200, 80)),
            Accent::Graphite => ((120, 120, 128), (142, 142, 147)),
        };
        let (r, g, b) = if dark { night } else { light };
        Color32::from_rgb(r, g, b)
    }
}

/// Wählbare Darstellungsgrößen (Zoomfaktor der gesamten Oberfläche).
pub const ZOOM_LEVELS: [(f32, &str); 4] = [
    (0.9, "Klein"),
    (1.0, "Normal"),
    (1.1, "Groß"),
    (1.25, "Sehr groß"),
];

/// Sprache der Oberfläche.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Language {
    #[default]
    German,
    English,
}

impl Language {
    pub const ALL: [Language; 2] = [Language::German, Language::English];

    pub fn label(self) -> &'static str {
        match self {
            Language::German => "Deutsch",
            Language::English => "English",
        }
    }
}

/// Gespeicherter Wecker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedAlarm {
    pub id: u64,
    pub hour: u8,
    pub minute: u8,
    pub label: String,
    pub enabled: bool,
    pub repeat: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub language: Language,
    pub appearance: Appearance,
    pub accent: Accent,
    /// Eigene Farben (RGB) – `None` = Standard des gewählten Designs.
    pub custom_accent: Option<[u8; 3]>,
    pub custom_background: Option<[u8; 3]>,
    pub custom_sidebar: Option<[u8; 3]>,
    pub custom_card: Option<[u8; 3]>,
    /// Eckenradius der Bedienelemente in Punkten (0 = eckig).
    pub rounding: f32,
    pub zoom: f32,
    /// Engere Abstände und kleinere Bedienelemente.
    pub compact: bool,
    /// Schlüssel des Moduls, das beim Start geöffnet wird; leer = zuletzt verwendetes.
    pub start_tab: String,
    pub last_tab: String,
    /// Reihenfolge der Werkzeuge in der Seitenleiste (Schlüssel); fehlende folgen am Ende.
    pub tab_order: Vec<String>,
    /// Ausgeblendete Werkzeuge (Schlüssel).
    pub hidden_tabs: Vec<String>,
    /// Spotify-Ansicht: Lautstärkeregler anzeigen.
    pub show_volume: bool,
    /// Schließen versteckt das Fenster im Tray, statt die App zu beenden.
    pub close_to_tray: bool,
    /// Im App-Starter auch Systemwerkzeuge, Hilfedateien u. Ä. anzeigen.
    pub show_system_apps: bool,
    /// Annahmen für die geschätzte Leistungsaufnahme (Watt bzw. Preis je kWh).
    pub power_cpu_max_w: f32,
    pub power_gpu_max_w: f32,
    pub power_extra_base_w: f32,
    pub power_price_kwh: f32,
    /// CO₂-Ausstoß des Strommixes in Gramm je kWh und Währungszeichen für die Kosten.
    pub power_co2_g_kwh: f32,
    pub power_currency: String,
    /// App-Starter: Favoriten, Sammlungen (Name, Pfade) und selbst hinzugefügte Einträge (Name, Pfad).
    pub launcher_favorites: Vec<String>,
    pub launcher_collections: Vec<(String, Vec<String>)>,
    pub launcher_custom: Vec<(String, String)>,
    /// Game Timer: eigene Programme (Name, Programmdatei) und ausgeschlossene Namen.
    pub games_custom: Vec<(String, String)>,
    pub games_ignored: Vec<String>,
    /// Uhr: Wecker, zuletzt eingestellter Countdown (Sekunden) und ob jede Taste den
    /// schwarzen Bildschirm beendet.
    pub alarms: Vec<SavedAlarm>,
    pub countdown_secs: u32,
    pub blackout_any_key: bool,

    /// Minecraft: Pfad zu MCA Selector (leer = automatisch suchen).
    pub mca_selector_path: String,
    /// Minecraft: zusätzliche Ordner (Instanz, `.minecraft` oder `saves`) mit Welten.
    pub minecraft_folders: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            language: Language::German,
            appearance: Appearance::System,
            accent: Accent::Blue,
            custom_accent: None,
            custom_background: None,
            custom_sidebar: None,
            custom_card: None,
            rounding: 4.0,
            zoom: 1.0,
            compact: false,
            start_tab: String::new(),
            last_tab: String::new(),
            tab_order: Vec::new(),
            hidden_tabs: Vec::new(),
            show_volume: true,
            close_to_tray: true,
            show_system_apps: false,
            power_cpu_max_w: 65.0,
            power_gpu_max_w: 150.0,
            power_extra_base_w: 0.0,
            power_price_kwh: 0.30,
            power_co2_g_kwh: 250.0,
            power_currency: "€".to_owned(),
            launcher_favorites: Vec::new(),
            launcher_collections: Vec::new(),
            launcher_custom: Vec::new(),
            games_custom: Vec::new(),
            games_ignored: Vec::new(),
            alarms: Vec::new(),
            countdown_secs: 300,
            blackout_any_key: false,
            mca_selector_path: String::new(),
            minecraft_folders: Vec::new(),
        }
    }
}

impl Settings {
    /// Listen des Game Timers für den Monitor-Worker.
    pub fn game_config(&self) -> crate::services::games::GameConfig {
        crate::services::games::GameConfig {
            custom: self.games_custom.clone(),
            ignored: self.games_ignored.clone(),
        }
    }

    /// Annahmen für die Leistungsschätzung des Monitors.
    pub fn power_config(&self) -> PowerConfig {
        PowerConfig {
            cpu_max_w: self.power_cpu_max_w.clamp(1.0, 1000.0),
            gpu_max_w: self.power_gpu_max_w.clamp(0.0, 1500.0),
            base_w: self.power_extra_base_w.clamp(0.0, 500.0),
        }
    }

    pub fn load(db: &Db) -> Self {
        db.get_setting(SETTING_KEY)
            .ok()
            .flatten()
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, db: &Db) -> AppResult<()> {
        let json = serde_json::to_string(self)
            .map_err(|e| AppError::other(format!("Einstellungen: {e}")))?;
        db.set_setting(SETTING_KEY, &json)
    }
}
