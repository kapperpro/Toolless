//! System-Monitor: Auslastung, Temperaturen, Prozesse, Datenträger, Netzwerk und
//! eine geschätzte Leistungsaufnahme – jeweils mit Verlauf.
//!
//! Ein Worker-Thread misst regelmäßig (auch wenn das Fenster im Tray liegt, dann
//! seltener) und legt die Werte in einem gemeinsamen Zustand ab, den die GUI liest.
//! Der Minutenverlauf der letzten Tage wird verschlüsselt gespeichert.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
        Arc, Mutex, MutexGuard,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use eframe::egui;
use serde::{Deserialize, Serialize};
use sysinfo::{Components, Disks, Networks, Pid, System};

use crate::{
    crypto,
    services::{
        games::{self, GameConfig, Library},
        gpu::GpuProbe,
        hwinfo::{self, SystemStats},
        pdh, perf_parse,
        winsys::{self, PhysicalDisk},
    },
};

/// Messabstand, solange der Monitor sichtbar ist bzw. im Hintergrund.
const DEFAULT_INTERVAL: Duration = Duration::from_secs(1);
const BACKGROUND_INTERVAL: Duration = Duration::from_secs(5);
/// Feiner Verlauf: eine Stunde im aktiven Messabstand.
const RECENT_CAPACITY: usize = 12_000;
/// Mittlerer Verlauf: zwei Tage in 30-Sekunden-Werten.
const THIRTY_CAPACITY: usize = 2 * 24 * 120;
/// Grober Verlauf: höchstens 30 Tage in Minutenwerten (die gewählte Dauer steht in den Einstellungen).
const MINUTES_MAX: usize = 30 * 24 * 60;
const SAVE_EVERY: Duration = Duration::from_secs(300);
const DISK_REFRESH_EVERY: Duration = Duration::from_secs(15);
/// Geschätzte Leistungsaufnahme des Arbeitsspeichers je Gigabyte.
const RAM_WATT_PER_GB: f32 = 0.35;
/// Anteil der Maximalleistung, den CPU bzw. GPU schon im Leerlauf aufnehmen.
const IDLE_SHARE: f32 = 0.1;
/// Auslastung aller 3D-Engines aller Grafikkarten (wie im Task-Manager).
const GPU_COUNTER: &str = r"\GPU Engine(*)\Utilization Percentage";
const GPU_MEMORY_COUNTER: &str = r"\GPU Adapter Memory(*)\Dedicated Usage";
const DISK_READ_COUNTER: &str = r"\PhysicalDisk(*)\Disk Read Bytes/sec";
const DISK_WRITE_COUNTER: &str = r"\PhysicalDisk(*)\Disk Write Bytes/sec";
const DISK_IDLE_COUNTER: &str = r"\PhysicalDisk(*)\% Idle Time";
/// Mittlere Antwortzeit je Datenträger (Sekunden je Übertragung).
const DISK_LATENCY_COUNTER: &str = r"\PhysicalDisk(*)\Avg. Disk sec/Transfer";
const GPU_SHARED_COUNTER: &str = r"\GPU Adapter Memory(*)\Shared Usage";
/// So lange (in Sekunden) bleibt die Auslastung der einzelnen Kerne im Speicher.
const CORE_HISTORY_SECONDS: f64 = 70.0;
/// Energiezähler von Windows (Milliwatt je Messstelle) – nur auf Systemen vorhanden,
/// deren Prozessor und Treiber sie bereitstellen.
const ENERGY_COUNTER: &str = r"\Energy Meter(*)\Power";
/// Nach so vielen Messungen wird der Indikator neu geöffnet, damit neu gestartete
/// Programme (neue Zählerinstanzen) erfasst werden.
const GPU_COUNTER_REBUILD: u32 = 30;
/// Mehr physische Datenträger werden nicht einzeln geführt.
const MAX_DISKS: usize = 16;

/// Ein Messpunkt des Verlaufs. Kurze Feldnamen halten die gespeicherte Datei klein.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Sample {
    /// Sekunden seit 1970.
    #[serde(rename = "t")]
    pub time: u64,
    /// Sekundenbruchteil des Zeitpunkts (für Messabstände unter einer Sekunde).
    #[serde(rename = "tf", default, skip_serializing_if = "is_zero")]
    pub frac: f32,
    /// Auslastungen in Prozent.
    #[serde(rename = "c")]
    pub cpu: f32,
    #[serde(rename = "r")]
    pub ram: f32,
    #[serde(rename = "g", default)]
    pub gpu: Option<f32>,
    /// Temperaturen in °C.
    #[serde(rename = "ct", default)]
    pub cpu_temp: Option<f32>,
    #[serde(rename = "gt", default)]
    pub gpu_temp: Option<f32>,
    /// Geschätzte Leistung in Watt: gesamt und je Komponente.
    #[serde(rename = "w", default)]
    pub power_w: f32,
    #[serde(rename = "wc", default)]
    pub cpu_w: f32,
    #[serde(rename = "wg", default)]
    pub gpu_w: f32,
    #[serde(rename = "wr", default)]
    pub ram_w: f32,
    /// Datenträger- und Netzwerkdurchsatz in Bytes je Sekunde.
    #[serde(rename = "dr", default)]
    pub disk_read: f32,
    #[serde(rename = "dw", default)]
    pub disk_write: f32,
    #[serde(rename = "nd", default)]
    pub net_down: f32,
    #[serde(rename = "nu", default)]
    pub net_up: f32,
    /// Programm mit der höchsten CPU-Last zu diesem Zeitpunkt.
    #[serde(rename = "a", default)]
    pub top_app: String,
    #[serde(rename = "ac", default)]
    pub top_cpu: f32,
    /// Je Grafikkarte: Auslastung in Prozent und Temperatur in °C (0 = unbekannt).
    #[serde(rename = "gs", default)]
    pub gpus: Vec<[f32; 2]>,
    /// Je physischem Datenträger: Lesen und Schreiben in Bytes/s, aktive Zeit in Prozent.
    #[serde(rename = "ds", default)]
    pub disks: Vec<[f32; 3]>,
}

fn is_zero(value: &f32) -> bool {
    *value == 0.0
}

impl Sample {
    /// Zeitpunkt in Sekunden seit 1970 samt Bruchteil.
    pub fn secs(&self) -> f64 {
        self.time as f64 + self.frac as f64
    }
}

/// Aktueller Stand einer Grafikkarte.
#[derive(Debug, Clone, Default)]
pub struct GpuInfo {
    pub name: String,
    pub usage: Option<f32>,
    pub temperature: Option<f32>,
    /// Belegter dedizierter Grafikspeicher in Bytes.
    pub memory_used: Option<u64>,
    /// Belegter gemeinsamer Speicher (Arbeitsspeicher, den die Karte nutzt) in Bytes.
    pub shared_used: Option<u64>,
}

/// Durchsatz einer Netzwerkschnittstelle in Bytes/s.
#[derive(Debug, Clone)]
pub struct NetworkRow {
    pub name: String,
    pub down: f32,
    pub up: f32,
}

/// Alle Prozesse eines Programms zusammengefasst (wie im Task-Manager).
#[derive(Debug, Clone)]
pub struct ProcessRow {
    /// Name der ausführbaren Datei, z. B. `chrome.exe`.
    pub name: String,
    /// Voller Pfad der Datei – für das Icon.
    pub exe: Option<String>,
    pub pids: Vec<u32>,
    /// CPU-Last in Prozent der Gesamtleistung.
    pub cpu: f32,
    /// Arbeitsspeicher in Bytes.
    pub memory: u64,
    /// Datenträgerdurchsatz in Bytes je Sekunde.
    pub disk_read: f32,
    pub disk_write: f32,
    /// Hat ein sichtbares Fenster („App“ statt Hintergrundprozess).
    pub is_app: bool,
    /// Geschätzter Anteil an der Leistungsaufnahme in Watt.
    pub power_w: f32,
}

#[derive(Debug, Clone)]
pub struct DiskRow {
    pub name: String,
    pub mount: String,
    pub file_system: String,
    pub kind: String,
    pub total: u64,
    pub free: u64,
    pub removable: bool,
}

/// Annahmen für die Leistungsschätzung (aus den Einstellungen).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PowerConfig {
    pub cpu_max_w: f32,
    pub gpu_max_w: f32,
    pub base_w: f32,
}

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct PowerBreakdown {
    pub cpu_w: f32,
    /// `true`, wenn Windows die Prozessorleistung misst (Energiezähler).
    pub cpu_measured: bool,
    pub gpu_w: f32,
    /// `true`: Der Treiber der Grafikkarte meldet die Leistung (NVIDIA, AMD).
    pub gpu_measured: bool,
    pub ram_w: f32,
    pub base_w: f32,
    pub total_w: f32,
    /// Seit dem Start der App verbrauchte Energie in Wattstunden.
    pub session_wh: f64,
    /// Seit Beginn der Aufzeichnung verbrauchte Energie in Wattstunden.
    pub total_wh: f64,
}

#[derive(Default)]
pub struct State {
    pub recent: VecDeque<Sample>,
    pub minutes: VecDeque<Sample>,
    pub thirty: VecDeque<Sample>,
    pub processes: Vec<ProcessRow>,
    pub disks: Vec<DiskRow>,
    pub physical_disks: Vec<PhysicalDisk>,

    pub power: PowerBreakdown,
    pub cpu_name: String,
    pub cpu_cores: usize,
    /// Auslastung je logischem Prozessor in Prozent.
    pub cores: Vec<f32>,
    pub cpu_mhz: u64,
    pub uptime: u64,
    /// Zähler des Systems (zugesichert, zwischengespeichert, Handles, Threads …).
    pub system: SystemStats,
    /// Auslastung je Kern über die letzten Sekunden: (Zeitpunkt, Prozent je Kern).
    pub core_history: VecDeque<(f64, Vec<f32>)>,
    /// Mittlere Antwortzeit je Datenträgernummer in Millisekunden.
    pub disk_latency: Vec<f32>,
    pub process_count: usize,
    pub swap_total: u64,
    pub swap_used: u64,
    pub gpus: Vec<GpuInfo>,
    /// Laufwerksbuchstaben je physischem Datenträger (Index = Datenträgernummer).
    pub disk_letters: Vec<String>,
    pub networks: Vec<NetworkRow>,
    /// Game Timer: alle erfassten Spiele und die Namen der gerade laufenden.
    pub games: Library,
    pub running_games: Vec<String>,
    /// Größe der Spielzeiten-Datei in Bytes (unverschlüsselt gerechnet).
    pub games_bytes: usize,
    pub total_ram: u64,
    pub used_ram: u64,
    /// Meldung des Workers (z. B. nach „Task beenden“); die GUI holt sie einmalig ab.
    pub message: Option<String>,
}

pub enum Command {
    /// Alle Prozesse (IDs) des genannten Programms beenden.
    Kill(String, Vec<u32>),
    Config(PowerConfig),
    RefreshDisks,
    /// Game Timer: eigene Programme und ausgeschlossene Namen.
    Games(GameConfig),
    /// Game Timer: ein Spiel samt seiner Zeiten löschen.
    RemoveGame(String),
    /// Messabstand in Millisekunden, solange der Task-Manager sichtbar ist.
    Interval(u64),
    /// So viele Tage bleibt der Verlauf gespeichert (1 bis 30).
    Retention(u32),
}

/// Gespeicherter Verlauf.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct HistoryFile {
    minutes: VecDeque<Sample>,
    thirty: VecDeque<Sample>,
    total_wh: f64,
}

/// Griff der GUI auf den Monitor-Worker.
pub struct MonitorHandle {
    tx: Sender<Command>,
    state: Arc<Mutex<State>>,
    active: Arc<AtomicBool>,
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl MonitorHandle {
    pub fn spawn(
        history_path: PathBuf,
        config: PowerConfig,
        game_config: GameConfig,
        repaint: egui::Context,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let state = Arc::new(Mutex::new(State::default()));
        let active = Arc::new(AtomicBool::new(false));

        let (worker_state, worker_active) = (Arc::clone(&state), Arc::clone(&active));
        let spawned = thread::Builder::new()
            .name("monitor".into())
            .spawn(move || {
                Worker::new(
                    rx,
                    worker_state,
                    worker_active,
                    history_path,
                    config,
                    game_config,
                    repaint,
                )
                .run();
            });
        if let Err(err) = spawned {
            lock(&state).message = Some(format!("Monitor konnte nicht starten: {err}"));
        }

        Self { tx, state, active }
    }

    pub fn send(&self, command: Command) {
        let _ = self.tx.send(command);
    }

    /// Liest den aktuellen Zustand (kurz halten – der Worker wartet so lange).
    pub fn read<T>(&self, f: impl FnOnce(&State) -> T) -> T {
        f(&lock(&self.state))
    }

    pub fn take_message(&self) -> Option<String> {
        lock(&self.state).message.take()
    }

    /// Schneller messen, solange der Monitor zu sehen ist.
    pub fn set_active(&self, active: bool) {
        self.active.store(active, Ordering::Relaxed);
    }
}

struct Worker {
    rx: Receiver<Command>,
    state: Arc<Mutex<State>>,
    active: Arc<AtomicBool>,
    history_path: PathBuf,
    config: PowerConfig,
    repaint: egui::Context,
    system: System,
    components: Components,
    networks: Networks,
    gpu: GpuProbe,
    gpu_counter: Option<pdh::Counter>,
    gpu_counter_age: u32,
    gpu_memory_counter: Option<pdh::Counter>,
    gpu_shared_counter: Option<pdh::Counter>,
    /// So viele Minutenwerte bleiben im Verlauf.
    minutes_capacity: usize,
    latency_counter: Option<pdh::Counter>,
    /// Größe des komprimierten Speichers (aus dem Prozess „Memory Compression“).
    compressed: u64,
    disk_counters: Option<[pdh::Counter; 3]>,
    energy_counter: Option<pdh::Counter>,
    /// Messpunkte der laufenden Minute (für den Minutenverlauf).
    minute_bucket: Vec<Sample>,
    thirty_bucket: Vec<Sample>,
    /// Messabstand bei sichtbarem Task-Manager.
    interval: Duration,
    /// Letzte vollständige Messung inklusive Prozessliste.
    last_full: Instant,
    last_sample: Instant,
    last_save: Instant,
    last_disks: Option<Instant>,
    session_wh: f64,
    total_wh: f64,
    games: Library,
    game_config: GameConfig,
    /// Noch nicht verbuchte Sekundenbruchteile der Spielzeit.
    game_carry: f64,
}

impl Worker {
    fn new(
        rx: Receiver<Command>,
        state: Arc<Mutex<State>>,
        active: Arc<AtomicBool>,
        history_path: PathBuf,
        config: PowerConfig,
        game_config: GameConfig,
        repaint: egui::Context,
    ) -> Self {
        Self {
            rx,
            state,
            active,
            history_path,
            config,
            repaint,
            system: System::new_all(),
            components: Components::new_with_refreshed_list(),
            networks: Networks::new_with_refreshed_list(),
            gpu: GpuProbe::new(),
            gpu_counter: pdh::Counter::open(GPU_COUNTER),
            gpu_counter_age: 0,
            gpu_memory_counter: pdh::Counter::open(GPU_MEMORY_COUNTER),
            gpu_shared_counter: pdh::Counter::open(GPU_SHARED_COUNTER),
            minutes_capacity: MINUTES_MAX,
            latency_counter: pdh::Counter::open(DISK_LATENCY_COUNTER),
            compressed: 0,

            disk_counters: open_disk_counters(),
            energy_counter: pdh::Counter::open(ENERGY_COUNTER),
            minute_bucket: Vec::new(),
            thirty_bucket: Vec::new(),
            interval: DEFAULT_INTERVAL,
            last_full: Instant::now()
                .checked_sub(Duration::from_secs(10))
                .unwrap_or_else(Instant::now),
            last_sample: Instant::now(),
            last_save: Instant::now(),
            last_disks: None,
            session_wh: 0.0,
            total_wh: 0.0,
            games: Library::default(),
            game_config,
            game_carry: 0.0,
        }
    }

    fn run(mut self) {
        self.load_history();
        self.load_games();

        {
            let mut state = lock(&self.state);
            state.cpu_name = self
                .system
                .cpus()
                .first()
                .map(|cpu| cpu.brand().trim().to_owned())
                .unwrap_or_default();
            state.cpu_cores = self.system.cpus().len();
        }

        loop {
            let interval = if self.active.load(Ordering::Relaxed) {
                self.interval
            } else {
                BACKGROUND_INTERVAL
            };
            match self.rx.recv_timeout(interval) {
                Ok(Command::Kill(name, pids)) => {
                    self.kill(&name, &pids);
                    continue; // die nächste Messung zeigt das Ergebnis
                }
                Ok(Command::Config(config)) => {
                    self.config = config;
                    continue;
                }
                Ok(Command::Retention(days)) => {
                    self.minutes_capacity = days.clamp(1, 30) as usize * 24 * 60;
                    let mut state = lock(&self.state);
                    while state.minutes.len() > self.minutes_capacity {
                        state.minutes.pop_front();
                    }
                    continue;
                }
                Ok(Command::Interval(millis)) => {
                    self.interval = Duration::from_millis(millis.clamp(100, 5000));
                    continue;
                }
                Ok(Command::Games(config)) => {
                    self.game_config = config;
                    self.save_games();
                    self.repaint.request_repaint();
                    continue;
                }
                Ok(Command::RemoveGame(name)) => {
                    self.games.remove(&name);
                    lock(&self.state).games = self.games.clone();
                    self.save_games();
                    self.repaint.request_repaint();
                    continue;
                }
                Ok(Command::RefreshDisks) => {
                    self.last_disks = None;
                    self.refresh_disks();
                    self.repaint.request_repaint();
                    continue;
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    self.save_history();
                    return;
                }
            }

            self.sample();
            self.refresh_disks();
            if self.last_save.elapsed() >= SAVE_EVERY {
                self.save_history();
            }
            if self.active.load(Ordering::Relaxed) {
                self.repaint.request_repaint();
            }
        }
    }

    fn kill(&mut self, name: &str, pids: &[u32]) {
        let killed = pids
            .iter()
            .filter(|pid| {
                self.system
                    .process(Pid::from_u32(**pid))
                    .is_some_and(|process| process.kill())
            })
            .count();
        let message = if killed == pids.len() {
            format!("„{name}“ beendet")
        } else {
            format!(
                "„{name}“: {killed} von {} Prozessen beendet (für den Rest fehlen die Rechte)",
                pids.len()
            )
        };
        lock(&self.state).message = Some(message);
        self.repaint.request_repaint();
    }

    /// Höchste plausible Temperatur der CPU-Sensoren, sofern das System welche meldet.
    fn cpu_temperature(&mut self) -> Option<f32> {
        let mut best: Option<f32> = None;
        for component in self.components.iter_mut() {
            component.refresh();
            let celsius: f32 = component.temperature();
            if celsius.is_finite() && celsius > 0.0 && celsius < 150.0 {
                best = Some(best.map_or(celsius, |current| current.max(celsius)));
            }
        }
        best
    }

    /// Auslastung je Grafikkarte (LUID) in Prozent; `None`, solange nichts messbar ist.
    fn gpu_usage(&mut self) -> Option<HashMap<u64, f32>> {
        self.gpu_counter_age += 1;
        // Neue Prozesse erscheinen erst in einem neu geöffneten Indikator.
        if self.gpu_counter_age >= GPU_COUNTER_REBUILD || self.gpu_counter.is_none() {
            self.gpu_counter_age = 0;
            self.gpu_counter = pdh::Counter::open(GPU_COUNTER);
            return None; // ein frisch geöffneter Indikator braucht zwei Messungen
        }
        let values = self.gpu_counter.as_mut()?.read();
        if values.is_empty() {
            return None;
        }
        Some(perf_parse::gpu_usage(&values))
    }

    /// Je physischem Datenträger: Lesen, Schreiben (Bytes/s), aktive Zeit (%) und Buchstaben.
    fn disk_activity(&mut self) -> (Vec<[f32; 3]>, Vec<String>) {
        let mut activity: Vec<[f32; 3]> = Vec::new();
        let mut letters: Vec<String> = Vec::new();
        let Some(counters) = self.disk_counters.as_mut() else {
            return (activity, letters);
        };
        for (slot, counter) in counters.iter_mut().enumerate() {
            for (name, value) in counter.read() {
                let Some((number, drive_letters)) = perf_parse::disk_instance(&name) else {
                    continue;
                };
                if number >= MAX_DISKS {
                    continue;
                }
                if activity.len() <= number {
                    activity.resize(number + 1, [0.0; 3]);
                    letters.resize(number + 1, String::new());
                }
                letters[number] = drive_letters.to_owned();
                activity[number][slot] = if slot == 2 {
                    // Gemessen wird die Leerlaufzeit.
                    (100.0 - value).clamp(0.0, 100.0) as f32
                } else {
                    value.max(0.0) as f32
                };
            }
        }
        (activity, letters)
    }

    /// Mittlere Antwortzeit je Datenträger in Millisekunden.
    fn disk_latency(&mut self) -> Vec<f32> {
        let mut latency: Vec<f32> = Vec::new();
        let Some(counter) = self.latency_counter.as_mut() else {
            return latency;
        };
        for (name, seconds) in counter.read() {
            let Some((number, _)) = perf_parse::disk_instance(&name) else {
                continue;
            };
            if number >= MAX_DISKS {
                continue;
            }
            if latency.len() <= number {
                latency.resize(number + 1, 0.0);
            }
            latency[number] = (seconds * 1000.0).max(0.0) as f32;
        }
        latency
    }

    /// Gemessene Leistung von Prozessor und Arbeitsspeicher in Watt, falls Windows
    /// Energiezähler dafür anbietet.
    fn measured_power(&mut self) -> (Option<f32>, Option<f32>) {
        let Some(counter) = self.energy_counter.as_mut() else {
            return (None, None);
        };
        let (mut package, mut memory): (Option<f32>, Option<f32>) = (None, None);
        for (name, milliwatts) in counter.read() {
            let watts = (milliwatts / 1000.0) as f32;
            if !(0.0..2000.0).contains(&watts) {
                continue;
            }
            let name = name.to_uppercase();
            if name.contains("PKG") {
                package = Some(package.unwrap_or(0.0) + watts);
            } else if name.contains("DRAM") {
                memory = Some(memory.unwrap_or(0.0) + watts);
            }
        }
        // Ein dauerhaftes 0 heißt „nicht unterstützt“, nicht „verbraucht nichts“.
        (package.filter(|w| *w > 0.2), memory.filter(|w| *w > 0.05))
    }

    fn sample(&mut self) {
        let elapsed = self.last_sample.elapsed();
        self.last_sample = Instant::now();
        let seconds = elapsed.as_secs_f32().max(0.1);

        // Bei sehr kurzen Messabständen werden nur Auslastung und Speicher neu gelesen; die
        // teure Prozessliste folgt höchstens einmal pro Sekunde.
        let light = self.last_full.elapsed() < Duration::from_millis(900);
        let full_seconds = self.last_full.elapsed().as_secs_f32().max(0.1);
        if light {
            self.system.refresh_cpu_usage();
            self.system.refresh_memory();
        } else {
            self.system.refresh_all();
            self.last_full = Instant::now();
        }
        let cores = self.system.cpus().len().max(1) as f32;
        let cpu = self.system.global_cpu_usage().clamp(0.0, 100.0);
        let total_ram = self.system.total_memory();
        let used_ram = self.system.used_memory();
        let ram = if total_ram > 0 {
            used_ram as f32 / total_ram as f32 * 100.0
        } else {
            0.0
        };

        // Netzwerkdurchsatz über alle Schnittstellen.
        self.networks.refresh();
        let (mut received, mut transmitted) = (0u64, 0u64);
        let mut networks: Vec<NetworkRow> = Vec::new();
        for (name, data) in &self.networks {
            received += data.received();
            transmitted += data.transmitted();
            // Nur Schnittstellen zeigen, über die je etwas lief.
            if data.total_received() + data.total_transmitted() > 0 {
                networks.push(NetworkRow {
                    name: name.clone(),
                    down: data.received() as f32 / seconds,
                    up: data.transmitted() as f32 / seconds,
                });
            }
        }
        networks.sort_by(|a, b| (b.down + b.up).total_cmp(&(a.down + a.up)));

        // Prozesse gleichen Namens zusammenfassen (bei leichten Messungen: letzter Stand).
        let (previous_processes, previous_games) = {
            let state = lock(&self.state);
            if light {
                (state.processes.clone(), state.running_games.clone())
            } else {
                (Vec::new(), Vec::new())
            }
        };
        let window_pids = if light {
            HashSet::new()
        } else {
            winsys::window_process_ids()
        };
        let mut groups: HashMap<String, ProcessRow> = HashMap::new();
        let live_processes = if light {
            None
        } else {
            Some(self.system.processes())
        };
        for (pid, process) in live_processes.into_iter().flatten() {
            let name = process.name().to_string_lossy().into_owned();
            if pid.as_u32() == 0 || name.is_empty() {
                continue; // Leerlaufprozess
            }
            let row = groups.entry(name.clone()).or_insert_with(|| ProcessRow {
                name,
                exe: None,
                pids: Vec::new(),
                cpu: 0.0,
                memory: 0,
                disk_read: 0.0,
                disk_write: 0.0,
                is_app: false,
                power_w: 0.0,
            });
            if row.exe.is_none() {
                row.exe = process
                    .exe()
                    .map(|path| path.to_string_lossy().into_owned());
            }
            let usage = process.disk_usage();
            row.pids.push(pid.as_u32());
            row.cpu += process.cpu_usage() / cores;
            row.memory += process.memory();
            row.disk_read += usage.read_bytes as f32 / full_seconds;
            row.disk_write += usage.written_bytes as f32 / full_seconds;
            row.is_app |= window_pids.contains(&pid.as_u32());
        }
        let mut processes: Vec<ProcessRow> = if light {
            previous_processes
        } else {
            groups.into_values().collect()
        };
        processes.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
        // Game Timer: Spielzeit der laufenden Spiele verbuchen (Pausen wie der
        // Ruhezustand zählen nicht mit).
        let mut running_games: Vec<(String, String)> = previous_games
            .into_iter()
            .map(|name| (name, String::new()))
            .collect();
        for row in processes.iter().filter(|_| !light) {
            let found = games::detect(&row.name, row.exe.as_deref(), row.is_app, &self.game_config);
            if let Some(name) = found {
                if !running_games.iter().any(|(known, _)| known == &name) {
                    running_games.push((name, row.exe.clone().unwrap_or_default()));
                }
            }
        }
        self.game_carry += elapsed.min(Duration::from_secs(30)).as_secs_f64();
        let played = self.game_carry.floor();
        self.game_carry -= played;
        if played >= 1.0 && !light {
            for (name, exe) in &running_games {
                self.games.record(name, exe, unix_now(), played as u64);
            }
        }

        // Datenträger: die Indikatoren von Windows sind genauer als die Summe der Prozesse.
        let (disk_activity, disk_letters) = self.disk_activity();
        let (disk_read, disk_write): (f32, f32) = if disk_activity.is_empty() {
            (
                processes.iter().map(|row| row.disk_read).sum(),
                processes.iter().map(|row| row.disk_write).sum(),
            )
        } else {
            (
                disk_activity.iter().map(|disk| disk[0]).sum(),
                disk_activity.iter().map(|disk| disk[1]).sum(),
            )
        };

        // Grafikkarten: Auslastung und Speicher aus den Indikatoren, Rest aus dem Treiber.
        let usage_by_adapter = self.gpu_usage();
        let memory_by_adapter = self
            .gpu_memory_counter
            .as_mut()
            .map(|counter| perf_parse::sum_by_adapter(&counter.read()))
            .unwrap_or_default();
        let shared_by_adapter = self
            .gpu_shared_counter
            .as_mut()
            .map(|counter| perf_parse::sum_by_adapter(&counter.read()))
            .unwrap_or_default();
        let gpu = self.gpu.sample();
        let gpus: Vec<GpuInfo> = gpu
            .adapters
            .iter()
            .enumerate()
            .map(|(index, adapter)| GpuInfo {
                name: if adapter.name.is_empty() {
                    format!("Grafikkarte {index}")
                } else {
                    adapter.name.clone()
                },
                usage: usage_by_adapter.as_ref().map(|usage| {
                    adapter
                        .luids
                        .iter()
                        .filter_map(|luid| usage.get(luid).copied())
                        .fold(0.0, f32::max)
                }),
                temperature: adapter.temperature,
                memory_used: adapter
                    .luids
                    .iter()
                    .filter_map(|luid| memory_by_adapter.get(luid))
                    .map(|bytes| *bytes as u64)
                    .max(),
                shared_used: adapter
                    .luids
                    .iter()
                    .filter_map(|luid| shared_by_adapter.get(luid))
                    .map(|bytes| *bytes as u64)
                    .max(),
            })
            .collect();
        // Ohne erkannte Karten bleibt wenigstens die Gesamtauslastung.
        let gpu_usage: Option<f32> = if gpus.is_empty() {
            usage_by_adapter
                .as_ref()
                .map(|usage| usage.values().copied().fold(0.0, f32::max))
        } else {
            gpus.iter().filter_map(|gpu| gpu.usage).reduce(f32::max)
        };
        let gpu_temp = gpus
            .iter()
            .filter_map(|gpu| gpu.temperature)
            .reduce(f32::max);
        let cpu_temp = self.cpu_temperature();
        let (measured_cpu_w, measured_ram_w) = self.measured_power();

        // ── Leistungsschätzung ──
        let config = self.config;
        let load_power = |max_w: f32, usage: f32| {
            max_w * (IDLE_SHARE + (1.0 - IDLE_SHARE) * usage.clamp(0.0, 100.0) / 100.0)
        };
        let (cpu_w, cpu_measured) = match measured_cpu_w {
            Some(measured) => (measured, true),
            None => (load_power(config.cpu_max_w, cpu), false),
        };
        let (gpu_w, gpu_measured) = match (gpu.power_w, gpu_usage) {
            (Some(measured), _) => (measured, true),
            (None, Some(usage)) => (load_power(config.gpu_max_w, usage), false),
            (None, None) => (config.gpu_max_w * IDLE_SHARE, false),
        };
        let ram_gb = total_ram as f32 / 1_073_741_824.0;
        let ram_w = measured_ram_w.unwrap_or(ram_gb * RAM_WATT_PER_GB * (0.5 + 0.5 * ram / 100.0));
        let total_w = cpu_w + gpu_w + ram_w + config.base_w;
        // Nach langen Pausen (Ruhezustand) nicht die ganze Zeit als Verbrauch zählen.
        let hours = elapsed.min(Duration::from_secs(30)).as_secs_f64() / 3600.0;
        self.session_wh += total_w as f64 * hours;
        self.total_wh += total_w as f64 * hours;

        // Der lastabhängige Teil der CPU-Leistung wird nach CPU-Anteil auf Programme verteilt,
        // der des Arbeitsspeichers nach belegtem Speicher.
        let idle_w = if cpu_measured {
            cpu_w * (1.0 - cpu / 100.0) * IDLE_SHARE
        } else {
            config.cpu_max_w * IDLE_SHARE
        };
        let dynamic_cpu_w = (cpu_w - idle_w).max(0.0);
        let cpu_sum: f32 = processes.iter().map(|row| row.cpu).sum();
        for row in &mut processes {
            let cpu_part = if cpu_sum > 0.0 {
                dynamic_cpu_w * row.cpu / cpu_sum
            } else {
                0.0
            };
            let ram_part = if total_ram > 0 {
                ram_w * row.memory as f32 / total_ram as f32
            } else {
                0.0
            };
            row.power_w = cpu_part + ram_part;
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let sample = Sample {
            time: now.as_secs(),
            frac: now.subsec_millis() as f32 / 1000.0,
            cpu,
            ram,
            gpu: gpu_usage,
            cpu_temp,
            gpu_temp,
            power_w: total_w,
            cpu_w,
            gpu_w,
            ram_w,
            disk_read,
            disk_write,
            net_down: received as f32 / seconds,
            net_up: transmitted as f32 / seconds,
            top_app: processes
                .first()
                .map(|row| row.name.clone())
                .unwrap_or_default(),
            top_cpu: processes.first().map_or(0.0, |row| row.cpu),
            gpus: gpus
                .iter()
                .map(|gpu| [gpu.usage.unwrap_or(0.0), gpu.temperature.unwrap_or(0.0)])
                .collect(),
            disks: disk_activity,
        };
        let cores: Vec<f32> = self
            .system
            .cpus()
            .iter()
            .map(|cpu| cpu.cpu_usage())
            .collect();
        let cpu_mhz = self.system.cpus().first().map_or(0, |cpu| cpu.frequency());
        if !light {
            self.compressed = processes
                .iter()
                .find(|row| row.name == "Memory Compression")
                .map_or(0, |row| row.memory);
        }
        let mut stats = hwinfo::system_stats().unwrap_or_default();
        stats.compressed = self.compressed;
        let disk_latency = self.disk_latency();
        let core_time = sample.secs();
        let process_count = self.system.processes().len();
        let (swap_total, swap_used) = (self.system.total_swap(), self.system.used_swap());
        let finished_minute = push_period(&mut self.minute_bucket, &sample, 60);
        let finished_thirty = push_period(&mut self.thirty_bucket, &sample, 30);

        let mut state = lock(&self.state);
        state.recent.push_back(sample);
        while state.recent.len() > RECENT_CAPACITY {
            state.recent.pop_front();
        }
        if let Some(entry) = finished_thirty {
            state.thirty.push_back(entry);
            while state.thirty.len() > THIRTY_CAPACITY {
                state.thirty.pop_front();
            }
        }
        if let Some(minute) = finished_minute {
            state.minutes.push_back(minute);
            while state.minutes.len() > self.minutes_capacity {
                state.minutes.pop_front();
            }
        }
        state.processes = processes;
        state.total_ram = total_ram;
        state.used_ram = used_ram;
        state.core_history.push_back((core_time, cores.clone()));
        while state
            .core_history
            .front()
            .is_some_and(|(time, _)| core_time - *time > CORE_HISTORY_SECONDS)
        {
            state.core_history.pop_front();
        }
        state.system = stats;
        if !disk_latency.is_empty() {
            state.disk_latency = disk_latency;
        }
        state.cores = cores;
        state.cpu_mhz = cpu_mhz;
        state.uptime = System::uptime();
        state.process_count = process_count;
        state.swap_total = swap_total;
        state.swap_used = swap_used;
        state.gpus = gpus;
        if !disk_letters.is_empty() {
            state.disk_letters = disk_letters;
        }
        state.networks = networks;
        state.games = self.games.clone();
        state.running_games = running_games.into_iter().map(|(name, _)| name).collect();
        state.power = PowerBreakdown {
            cpu_w,
            cpu_measured,
            gpu_w,
            gpu_measured,
            ram_w,
            base_w: config.base_w,
            total_w,
            session_wh: self.session_wh,
            total_wh: self.total_wh,
        };
    }

    /// Volumes, physische Datenträger und Autostart-Einträge – ändern sich selten.
    fn refresh_disks(&mut self) {
        if self
            .last_disks
            .is_some_and(|at| at.elapsed() < DISK_REFRESH_EVERY)
        {
            return;
        }
        self.last_disks = Some(Instant::now());
        let disks: Vec<DiskRow> = Disks::new_with_refreshed_list()
            .iter()
            .map(|disk| DiskRow {
                name: disk.name().to_string_lossy().into_owned(),
                mount: disk.mount_point().to_string_lossy().into_owned(),
                file_system: disk.file_system().to_string_lossy().into_owned(),
                kind: disk.kind().to_string(),
                total: disk.total_space(),
                free: disk.available_space(),
                removable: disk.is_removable(),
            })
            .collect();
        let physical_disks = winsys::physical_disks();

        let mut state = lock(&self.state);
        state.disks = disks;
        state.physical_disks = physical_disks;
    }

    fn load_history(&mut self) {
        let Ok(bytes) = crypto::read_file(&self.history_path) else {
            return;
        };
        // Aktuelles Format; ältere Dateien enthielten nur die Liste der Minutenwerte.
        let history = serde_json::from_slice::<HistoryFile>(&bytes).or_else(|_| {
            serde_json::from_slice::<VecDeque<Sample>>(&bytes).map(|minutes| HistoryFile {
                minutes,
                thirty: VecDeque::new(),
                total_wh: 0.0,
            })
        });
        if let Ok(mut history) = history {
            // Nur behalten, was noch in den Zeitraum fällt.
            let oldest = unix_now().saturating_sub(MINUTES_MAX as u64 * 60);
            history.minutes.retain(|sample| sample.time >= oldest);
            let oldest_thirty = unix_now().saturating_sub(THIRTY_CAPACITY as u64 * 30);
            history.thirty.retain(|sample| sample.time >= oldest_thirty);
            self.total_wh = history.total_wh;
            let mut state = lock(&self.state);
            state.minutes = history.minutes;
            state.thirty = history.thirty;
            state.power.total_wh = self.total_wh;
        }
    }

    fn games_path(&self) -> PathBuf {
        self.history_path.with_file_name("games.dat")
    }

    fn load_games(&mut self) {
        let Ok(bytes) = crypto::read_file(&self.games_path()) else {
            return;
        };
        if let Ok(library) = serde_json::from_slice::<Library>(&bytes) {
            self.games = library;
            let mut state = lock(&self.state);
            state.games = self.games.clone();
            state.games_bytes = bytes.len();
        }
    }

    fn save_games(&mut self) {
        let size_of = |library: &Library| serde_json::to_vec(library).map_or(0, |json| json.len());
        self.games.prune(
            unix_now(),
            self.game_config.retention_secs(),
            self.game_config.max_bytes(),
            size_of,
        );
        if let Ok(json) = serde_json::to_vec(&self.games) {
            let _ = crypto::write_file(&self.games_path(), &json);
            let mut state = lock(&self.state);
            state.games = self.games.clone();
            state.games_bytes = json.len();
        }
    }

    fn save_history(&mut self) {
        self.last_save = Instant::now();
        self.save_games();
        let json = {
            let state = lock(&self.state);
            serde_json::to_vec(&HistoryFile {
                minutes: state.minutes.clone(),
                thirty: state.thirty.clone(),
                total_wh: self.total_wh,
            })
        };
        if let Ok(json) = json {
            // Schlägt das Schreiben fehl, wird es beim nächsten Mal erneut versucht.
            let _ = crypto::write_file(&self.history_path, &json);
        }
    }
}

/// Sammelt Messpunkte je Zeitraum (`period` Sekunden); liefert den Mittelwert, sobald ein
/// Zeitraum voll ist.
fn push_period(bucket: &mut Vec<Sample>, sample: &Sample, period: u64) -> Option<Sample> {
    let slot = sample.time / period;
    let finished = match bucket.first() {
        Some(first) if first.time / period != slot => Some(average(bucket)),
        _ => None,
    };
    if finished.is_some() {
        bucket.clear();
    }
    bucket.push(sample.clone());
    finished
}

/// Mittelwert mehrerer Messpunkte; als „Top-Programm“ gilt das mit der höchsten Last.
fn average(samples: &[Sample]) -> Sample {
    let count = samples.len().max(1) as f32;
    let mean = |value: fn(&Sample) -> f32| samples.iter().map(value).sum::<f32>() / count;
    let mean_some = |value: fn(&Sample) -> Option<f32>| {
        let values: Vec<f32> = samples.iter().filter_map(value).collect();
        if values.is_empty() {
            None
        } else {
            Some(values.iter().sum::<f32>() / values.len() as f32)
        }
    };
    let top = samples
        .iter()
        .max_by(|a, b| a.top_cpu.total_cmp(&b.top_cpu));
    Sample {
        time: samples.first().map_or(0, |sample| sample.time),
        frac: samples.first().map_or(0.0, |sample| sample.frac),
        cpu: mean(|s| s.cpu),
        ram: mean(|s| s.ram),
        gpu: mean_some(|s| s.gpu),
        cpu_temp: mean_some(|s| s.cpu_temp),
        gpu_temp: mean_some(|s| s.gpu_temp),
        power_w: mean(|s| s.power_w),
        cpu_w: mean(|s| s.cpu_w),
        gpu_w: mean(|s| s.gpu_w),
        ram_w: mean(|s| s.ram_w),
        disk_read: mean(|s| s.disk_read),
        disk_write: mean(|s| s.disk_write),
        net_down: mean(|s| s.net_down),
        net_up: mean(|s| s.net_up),
        top_app: top.map(|s| s.top_app.clone()).unwrap_or_default(),
        top_cpu: top.map_or(0.0, |s| s.top_cpu),
        gpus: mean_rows(samples, |s| &s.gpus, true),
        disks: mean_rows(samples, |s| &s.disks, false),
    }
}

/// Mittelt Wertezeilen (je Grafikkarte bzw. Datenträger) über mehrere Messpunkte.
/// Mit `skip_zero` zählen Nullen ab der zweiten Spalte als „kein Wert“ (Temperaturen).
fn mean_rows<const N: usize>(
    samples: &[Sample],
    rows: fn(&Sample) -> &Vec<[f32; N]>,
    skip_zero: bool,
) -> Vec<[f32; N]> {
    let len = samples
        .iter()
        .map(|sample| rows(sample).len())
        .max()
        .unwrap_or(0);
    (0..len)
        .map(|index| {
            let mut mean = [0.0f32; N];
            for (column, slot) in mean.iter_mut().enumerate() {
                let values: Vec<f32> = samples
                    .iter()
                    .filter_map(|sample| rows(sample).get(index).map(|row| row[column]))
                    .filter(|value| !(skip_zero && column > 0 && *value == 0.0))
                    .collect();
                if !values.is_empty() {
                    *slot = values.iter().sum::<f32>() / values.len() as f32;
                }
            }
            mean
        })
        .collect()
}

fn open_disk_counters() -> Option<[pdh::Counter; 3]> {
    Some([
        pdh::Counter::open(DISK_READ_COUNTER)?,
        pdh::Counter::open(DISK_WRITE_COUNTER)?,
        pdh::Counter::open(DISK_IDLE_COUNTER)?,
    ])
}
