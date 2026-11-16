//! System-Monitor: Auslastung, Temperaturen, Prozesse, Datenträger, Netzwerk und
//! eine geschätzte Leistungsaufnahme – jeweils mit Verlauf.
//!
//! Ein Worker-Thread misst regelmäßig (auch wenn das Fenster im Tray liegt, dann
//! seltener) und legt die Werte in einem gemeinsamen Zustand ab, den die GUI liest.
//! Der Minutenverlauf der letzten Tage wird verschlüsselt gespeichert.

use std::{
    collections::{HashMap, VecDeque},
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
        pdh, perf_parse,
        winsys::{self, PhysicalDisk},
    },
};

/// Messabstand, solange der Monitor sichtbar ist bzw. im Hintergrund.
const ACTIVE_INTERVAL: Duration = Duration::from_secs(2);
const BACKGROUND_INTERVAL: Duration = Duration::from_secs(10);
/// Feiner Verlauf: eine Stunde im aktiven Messabstand.
const RECENT_CAPACITY: usize = 1800;
/// Grober Verlauf: sieben Tage in Minutenwerten.
const MINUTES_CAPACITY: usize = 7 * 24 * 60;
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

/// Aktueller Stand einer Grafikkarte.
#[derive(Debug, Clone, Default)]
pub struct GpuInfo {
    pub name: String,
    pub usage: Option<f32>,
    pub temperature: Option<f32>,
    /// Belegter dedizierter Grafikspeicher in Bytes.
    pub memory_used: Option<u64>,
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
}

/// Gespeicherter Verlauf.
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct HistoryFile {
    minutes: VecDeque<Sample>,
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
    disk_counters: Option<[pdh::Counter; 3]>,
    energy_counter: Option<pdh::Counter>,
    /// Messpunkte der laufenden Minute (für den Minutenverlauf).
    minute_bucket: Vec<Sample>,
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
            disk_counters: open_disk_counters(),
            energy_counter: pdh::Counter::open(ENERGY_COUNTER),
            minute_bucket: Vec::new(),
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
                ACTIVE_INTERVAL
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
                Ok(Command::Games(config)) => {
                    self.game_config = config;
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

        self.system.refresh_all();
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

        // Prozesse gleichen Namens zusammenfassen.
        let window_pids = winsys::window_process_ids();
        let mut groups: HashMap<String, ProcessRow> = HashMap::new();
        for (pid, process) in self.system.processes() {
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
            row.disk_read += usage.read_bytes as f32 / seconds;
            row.disk_write += usage.written_bytes as f32 / seconds;
            row.is_app |= window_pids.contains(&pid.as_u32());
        }
        let mut processes: Vec<ProcessRow> = groups.into_values().collect();
        processes.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
        // Game Timer: Spielzeit der laufenden Spiele verbuchen (Pausen wie der
        // Ruhezustand zählen nicht mit).
        let mut running_games: Vec<(String, String)> = Vec::new();
        for row in &processes {
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
        if played >= 1.0 {
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

        let sample = Sample {
            time: unix_now(),
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
        let process_count = self.system.processes().len();
        let (swap_total, swap_used) = (self.system.total_swap(), self.system.used_swap());
        let finished_minute = self.push_minute(&sample);

        let mut state = lock(&self.state);
        state.recent.push_back(sample);
        while state.recent.len() > RECENT_CAPACITY {
            state.recent.pop_front();
        }
        if let Some(minute) = finished_minute {
            state.minutes.push_back(minute);
            while state.minutes.len() > MINUTES_CAPACITY {
                state.minutes.pop_front();
            }
        }
        state.processes = processes;
        state.total_ram = total_ram;
        state.used_ram = used_ram;
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

    /// Sammelt Messpunkte je Minute; liefert den Mittelwert, sobald eine Minute voll ist.
    fn push_minute(&mut self, sample: &Sample) -> Option<Sample> {
        let minute = sample.time / 60;
        let finished = match self.minute_bucket.first() {
            Some(first) if first.time / 60 != minute => Some(average(&self.minute_bucket)),
            _ => None,
        };
        if finished.is_some() {
            self.minute_bucket.clear();
        }
        self.minute_bucket.push(sample.clone());
        finished
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
                total_wh: 0.0,
            })
        });
        if let Ok(mut history) = history {
            // Nur behalten, was noch in den Zeitraum fällt.
            let oldest = unix_now().saturating_sub(MINUTES_CAPACITY as u64 * 60);
            history.minutes.retain(|sample| sample.time >= oldest);
            self.total_wh = history.total_wh;
            let mut state = lock(&self.state);
            state.minutes = history.minutes;
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
            lock(&self.state).games = self.games.clone();
        }
    }

    fn save_games(&self) {
        if let Ok(json) = serde_json::to_vec(&self.games) {
            let _ = crypto::write_file(&self.games_path(), &json);
        }
    }

    fn save_history(&mut self) {
        self.last_save = Instant::now();
        self.save_games();
        let json = {
            let state = lock(&self.state);
            serde_json::to_vec(&HistoryFile {
                minutes: state.minutes.clone(),
                total_wh: self.total_wh,
            })
        };
        if let Ok(json) = json {
            // Schlägt das Schreiben fehl, wird es beim nächsten Mal erneut versucht.
            let _ = crypto::write_file(&self.history_path, &json);
        }
    }
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
