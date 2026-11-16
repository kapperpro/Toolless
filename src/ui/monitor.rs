use crate::i18n::t;
use std::{
    collections::HashMap,
    path::Path,
    sync::mpsc::{self, Receiver, Sender},
    thread,
};

use eframe::egui::{
    self, Align, Align2, Color32, FontId, Layout, Pos2, Rect, RichText, Sense, Shape, Stroke,
    TextEdit, TextureHandle, Ui,
};

use crate::{
    app::Shared,
    platform,
    services::{
        icons::{self, IconImage},
        monitor::{
            Command, DiskRow, GpuInfo, NetworkRow, PowerBreakdown, ProcessRow, Sample, State,
        },
        winsys::PhysicalDisk,
    },
    theme::{self, Palette},
};

/// So viele Punkte zeichnet ein Diagramm höchstens – mehr sieht man ohnehin nicht.
const MAX_CHART_POINTS: usize = 600;
const CHART_HEIGHT: f32 = 170.0;
const TIMELINE_HEIGHT: f32 = 64.0;
const PROCESS_ROW_HEIGHT: f32 = 28.0;
const MAX_PROCESS_ROWS: usize = 300;
const BYTES_PER_MB: f32 = 1_048_576.0;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Page {
    #[default]
    Processes,
    Performance,
    History,
    Disks,
    Energy,
}

impl Page {
    const ALL: [(Page, &'static str); 5] = [
        (Page::Processes, "Prozesse"),
        (Page::Performance, "Leistung"),
        (Page::History, "Verlauf"),
        (Page::Disks, "Datenträger"),
        (Page::Energy, "Energie"),
    ];
}

/// Zeitraum der Diagramme.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Range {
    Minute,
    #[default]
    FiveMinutes,
    Hour,
    Day,
    Week,
}

impl Range {
    const ALL: [Range; 5] = [
        Range::Minute,
        Range::FiveMinutes,
        Range::Hour,
        Range::Day,
        Range::Week,
    ];

    fn label(self) -> &'static str {
        match self {
            Range::Minute => "60 Sekunden",
            Range::FiveMinutes => "5 Minuten",
            Range::Hour => "1 Stunde",
            Range::Day => "24 Stunden",
            Range::Week => "7 Tage",
        }
    }

    fn seconds(self) -> u64 {
        match self {
            Range::Minute => 60,
            Range::FiveMinutes => 300,
            Range::Hour => 3600,
            Range::Day => 86_400,
            Range::Week => 7 * 86_400,
        }
    }

    /// Zeitraum der Zeitleiste unter dem Verlauf – jeweils deutlich größer als das Fenster.
    fn overview_seconds(self) -> u64 {
        match self {
            Range::Minute => 3600,
            Range::FiveMinutes | Range::Hour => 86_400,
            Range::Day | Range::Week => 7 * 86_400,
        }
    }

    /// Lange Zeiträume kommen aus dem gespeicherten Minutenverlauf.
    fn uses_minutes(self) -> bool {
        matches!(self, Range::Day | Range::Week)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Sort {
    #[default]
    Cpu,
    Memory,
    Disk,
    Name,
}

/// Gerät auf den Seiten „Leistung“ und „Verlauf“ (wie die linke Liste im Task-Manager).
/// Grafikkarten und Datenträger tragen ihre Nummer.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Device {
    #[default]
    Cpu,
    Memory,
    Gpu(usize),
    Disk(usize),
    Network,
}

/// Lädt Programm-Icons in einem Hintergrund-Thread, damit die Liste nicht stockt.
struct IconLoader {
    requests: Sender<String>,
    results: Receiver<(String, Option<IconImage>)>,
}

impl IconLoader {
    fn spawn(ctx: egui::Context) -> Self {
        let (requests, incoming) = mpsc::channel::<String>();
        let (outgoing, results) = mpsc::channel();
        thread::spawn(move || {
            let _com = icons::ComGuard::new();
            for path in incoming {
                let icon = icons::extract(Path::new(&path));
                if outgoing.send((path, icon)).is_err() {
                    break;
                }
                ctx.request_repaint();
            }
        });
        Self { requests, results }
    }
}

#[derive(Default)]
pub struct MonitorView {
    page: Page,
    range: Range,
    sort: Sort,
    device: Device,
    /// Gewählte Komponente auf der Seite „Verlauf“.
    history_device: Device,
    /// Verlauf: so viele Sekunden liegt das Ende des gezeigten Fensters zurück (0 = live).
    history_offset: u64,

    search: String,
    /// Name des markierten Programms in der Prozessliste.
    selected: Option<String>,
    /// Wartet auf Bestätigung: Programm (Name, Prozess-IDs) beenden.
    confirm_kill: Option<(String, Vec<u32>)>,
    /// Icons je Programmdatei; `None` = angefragt oder nicht vorhanden.
    icons: HashMap<String, Option<TextureHandle>>,
    icon_loader: Option<IconLoader>,
    /// Anzahl der Zeilen in der Energie-Tabelle.
    energy_rows: usize,
}

// ───────── Hilfsfunktionen ─────────

/// Kleine Werte mit Nachkommastelle, große gerundet.
fn format_value(value: f32, max: f32) -> String {
    if max < 20.0 {
        format!("{value:.1}").replace('.', ",")
    } else {
        format!("{value:.0}")
    }
}

/// Durchsatz in MB/s (bzw. KB/s bei kleinen Werten).
fn format_rate(bytes_per_second: f32) -> String {
    if bytes_per_second >= BYTES_PER_MB / 10.0 {
        format!("{:.1} MB/s", bytes_per_second / BYTES_PER_MB).replace('.', ",")
    } else {
        format!("{:.0} KB/s", bytes_per_second / 1024.0)
    }
}

fn format_percent(value: f32) -> String {
    format!("{value:.1} %").replace('.', ",")
}

/// Programmname ohne die Endung `.exe`.
fn display_name(name: &str) -> &str {
    let lower = name.to_lowercase();
    if lower.ends_with(".exe") {
        &name[..name.len() - 4]
    } else {
        name
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn format_bytes(bytes: u64) -> String {
    const GB: f64 = 1_073_741_824.0;
    let gigabytes = bytes as f64 / GB;
    if gigabytes >= 1024.0 {
        format!("{:.2} TB", gigabytes / 1024.0).replace('.', ",")
    } else if gigabytes >= 1.0 {
        format!("{gigabytes:.1} GB").replace('.', ",")
    } else {
        format!("{:.0} MB", bytes as f64 / 1_048_576.0)
    }
}

/// „vor 12 min“, „vor 3 h 20 min“ …
fn format_ago(seconds: u64) -> String {
    match seconds {
        0..=4 => "jetzt".to_owned(),
        5..=119 => crate::i18n::td(format!("vor {seconds} s")),
        120..=7199 => crate::i18n::td(format!("vor {} min", seconds / 60)),
        7200..=172_799 => crate::i18n::td(format!(
            "vor {} h {} min",
            seconds / 3600,
            seconds % 3600 / 60
        )),
        _ => crate::i18n::td(format!(
            "vor {} T {} h",
            seconds / 86_400,
            seconds % 86_400 / 3600
        )),
    }
}

/// Wählt die Messpunkte des Zeitraums und dünnt sie auf eine zeichenbare Menge aus.
fn collect_samples(state: &State, range: Range, now: u64) -> Vec<Sample> {
    collect_between(
        state,
        now.saturating_sub(range.seconds()),
        now,
        range.uses_minutes(),
    )
}

/// Messpunkte zwischen zwei Zeitpunkten. Die feinen Werte reichen nur etwa eine Stunde
/// zurück – für alles Ältere (oder mit `coarse`) dient der Minutenverlauf.
fn collect_between(state: &State, start: u64, end: u64, coarse: bool) -> Vec<Sample> {
    let recent_from = state.recent.front().map_or(u64::MAX, |sample| sample.time);
    let source = if coarse || start + 60 < recent_from {
        &state.minutes
    } else {
        &state.recent
    };
    let in_range: Vec<&Sample> = source
        .iter()
        .filter(|sample| sample.time >= start && sample.time <= end)
        .collect();
    let step = in_range.len().div_ceil(MAX_CHART_POINTS).max(1);
    let mut samples: Vec<Sample> = in_range
        .iter()
        .step_by(step)
        .map(|sample| (*sample).clone())
        .collect();
    // Der neueste Punkt soll immer dabei sein.
    if let (Some(last), Some(newest)) = (samples.last(), in_range.last()) {
        if last.time != newest.time {
            samples.push((*newest).clone());
        }
    }
    samples
}

/// Uhrzeit eines Zeitstempels in Ortszeit, auf Wunsch mit Datum.
fn format_clock(time: u64, with_date: bool) -> String {
    let local = time as i64 + platform::local_offset_secs();
    let (days, seconds) = (local.div_euclid(86_400), local.rem_euclid(86_400));
    let clock = format!("{:02}:{:02}", seconds / 3600, seconds % 3600 / 60);
    if !with_date {
        return clock;
    }
    // Tage seit 1970 → Kalenderdatum (Algorithmus nach Howard Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let _ = era;
    format!("{day:02}.{month:02}. {clock}")
}

/// Füllt die Fläche unter einem Linienzug bis `bottom` mit einer zarten Farbe.
fn fill_below(painter: &egui::Painter, points: &[Pos2], bottom: f32, color: Color32) {
    if points.len() < 2 {
        return;
    }
    let mut mesh = egui::Mesh::default();
    for pair in points.windows(2) {
        let first = mesh.vertices.len() as u32;
        mesh.colored_vertex(pair[0], color);
        mesh.colored_vertex(pair[1], color);
        mesh.colored_vertex(Pos2::new(pair[1].x, bottom), color);
        mesh.colored_vertex(Pos2::new(pair[0].x, bottom), color);
        mesh.add_triangle(first, first + 1, first + 2);
        mesh.add_triangle(first, first + 2, first + 3);
    }
    painter.add(Shape::mesh(mesh));
}

/// Eine Linie im Diagramm.
struct Line<'a> {
    label: &'a str,
    color: Color32,
    value: &'a dyn Fn(&Sample) -> Option<f32>,
}

/// Liniendiagramm über die Zeit mit Legende und Werten unter dem Mauszeiger.
fn chart(
    ui: &mut Ui,
    p: &Palette,
    samples: &[Sample],
    lines: &[Line],
    max: f32,
    unit: &str,
    range: Range,
    now: u64,
) {
    // Legende mit den aktuellen Werten.
    ui.horizontal_wrapped(|ui| {
        for line in lines {
            let (dot, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
            ui.painter().circle_filled(dot.center(), 4.0, line.color);
            let latest = samples.last().and_then(line.value);
            let text = match latest {
                Some(value) => format!("{} {} {unit}", line.label, format_value(value, max)),
                None => format!("{} –", line.label),
            };
            ui.label(theme::secondary(p, text).size(12.5));
            ui.add_space(8.0);
        }
    });

    let size = egui::vec2(ui.available_width(), CHART_HEIGHT);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, p.radius, p.field_bg);

    let plot = rect.shrink2(egui::vec2(8.0, 10.0));
    let span = range.seconds() as f32;
    let start = now.saturating_sub(range.seconds());
    let x_of = |time: u64| {
        plot.left() + (time.saturating_sub(start) as f32 / span).clamp(0.0, 1.0) * plot.width()
    };
    let y_of = |value: f32| plot.bottom() - (value / max).clamp(0.0, 1.0) * plot.height();

    // Hilfslinien bei 25 / 50 / 75 / 100 %.
    let grid = Stroke::new(1.0_f32, p.separator.gamma_multiply(0.5));
    for quarter in 1..=4 {
        let value = max * quarter as f32 / 4.0;
        let y = y_of(value);
        painter.hline(plot.x_range(), y, grid);
        painter.text(
            Pos2::new(plot.right(), y + 2.0),
            Align2::RIGHT_TOP,
            format!("{} {unit}", format_value(value, max)),
            FontId::proportional(10.5),
            p.text_secondary,
        );
    }

    // Größere Lücken (App war aus, Rechner im Ruhezustand) nicht überbrücken.
    let max_gap = (range.seconds() / 40).max(if range.uses_minutes() { 180 } else { 25 });
    for line in lines {
        let mut segment: Vec<Pos2> = Vec::new();
        let mut previous_time = 0u64;
        let flush = |segment: &mut Vec<Pos2>| {
            if segment.len() > 1 {
                fill_below(
                    &painter,
                    segment,
                    plot.bottom(),
                    line.color.gamma_multiply(0.16),
                );
                painter.add(Shape::line(
                    std::mem::take(segment),
                    Stroke::new(1.6_f32, line.color),
                ));
            } else if let Some(point) = segment.pop() {
                painter.circle_filled(point, 1.5, line.color);
            }
        };
        for sample in samples {
            match (line.value)(sample) {
                Some(value) => {
                    if !segment.is_empty() && sample.time.saturating_sub(previous_time) > max_gap {
                        flush(&mut segment);
                    }
                    segment.push(Pos2::new(x_of(sample.time), y_of(value)));
                    previous_time = sample.time;
                }
                None => flush(&mut segment),
            }
        }
        flush(&mut segment);
    }

    // Zeitachse: beim Blick in die Vergangenheit mit Uhrzeiten statt „vor …“.
    let real_now = unix_now();
    let shifted = real_now.saturating_sub(now) > 5;
    let with_date = real_now.saturating_sub(start) > 86_400;
    let axis_font = FontId::proportional(10.5);
    painter.text(
        Pos2::new(plot.left(), rect.bottom() - 1.0),
        Align2::LEFT_BOTTOM,
        if shifted {
            format_clock(start, with_date)
        } else {
            format_ago(range.seconds())
        },
        axis_font.clone(),
        p.text_secondary,
    );
    painter.text(
        Pos2::new(plot.right(), rect.bottom() - 1.0),
        Align2::RIGHT_BOTTOM,
        if shifted {
            format_clock(now, with_date)
        } else {
            "jetzt".to_owned()
        },
        axis_font,
        p.text_secondary,
    );

    // Werte unter dem Mauszeiger – samt dem Programm, das zu der Zeit am meisten Last erzeugte.
    let Some(pointer) = response.hover_pos() else {
        return;
    };
    let nearest = samples.iter().min_by(|a, b| {
        (x_of(a.time) - pointer.x)
            .abs()
            .total_cmp(&(x_of(b.time) - pointer.x).abs())
    });
    let Some(sample) = nearest else {
        return;
    };
    painter.vline(
        x_of(sample.time),
        plot.y_range(),
        Stroke::new(1.0_f32, p.text_secondary),
    );
    response.on_hover_ui_at_pointer(|ui| {
        let when = format!(
            "{} · {}",
            format_clock(sample.time, with_date),
            format_ago(real_now.saturating_sub(sample.time))
        );
        ui.label(RichText::new(when).strong());
        for line in lines {
            if let Some(value) = (line.value)(sample) {
                ui.label(format!(
                    "{}: {} {unit}",
                    line.label,
                    format_value(value, max)
                ));
            }
        }
        if !sample.top_app.is_empty() {
            ui.label(
                theme::secondary(
                    p,
                    crate::i18n::td(format!(
                        "Aktivstes Programm: {} ({:.0} % CPU)",
                        sample.top_app, sample.top_cpu
                    )),
                )
                .size(12.0),
            );
        }
    });
}

fn series_colors(p: &Palette) -> (Color32, Color32, Color32) {
    let second = if p.dark {
        Color32::from_rgb(255, 170, 60)
    } else {
        Color32::from_rgb(200, 110, 0)
    };
    let third = if p.dark {
        Color32::from_rgb(120, 210, 120)
    } else {
        Color32::from_rgb(30, 140, 60)
    };
    (p.accent, second, third)
}

/// Kleines Verlaufsbild für die Geräteliste.
fn sparkline(
    ui: &Ui,
    p: &Palette,
    rect: Rect,
    samples: &[Sample],
    value: &dyn Fn(&Sample) -> Option<f32>,
    max: f32,
    color: Color32,
) {
    ui.painter()
        .rect_stroke(rect, 2.0, Stroke::new(1.0_f32, p.separator));
    let values: Vec<f32> = samples.iter().filter_map(value).collect();
    if values.len() < 2 {
        return;
    }
    let step = rect.width() / (values.len() - 1) as f32;
    let points: Vec<Pos2> = values
        .iter()
        .enumerate()
        .map(|(index, v)| {
            Pos2::new(
                rect.left() + index as f32 * step,
                rect.bottom() - (v / max.max(0.001)).clamp(0.0, 1.0) * rect.height(),
            )
        })
        .collect();
    let painter = ui.painter_at(rect);
    fill_below(&painter, &points, rect.bottom(), color.gamma_multiply(0.18));
    painter.add(Shape::line(points, Stroke::new(1.2_f32, color)));
}

/// Beschriftung über einem Wert, wie in den Detailfeldern des Task-Managers.
fn detail(ui: &mut Ui, p: &Palette, label: &str, value: String) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.label(theme::secondary(p, label).size(12.0));
        ui.label(RichText::new(value).size(17.0));
    });
    ui.add_space(18.0);
}

/// Höchster Wert einer Reihe, mit etwas Luft und Untergrenze – als Diagramm-Obergrenze.
fn headroom(samples: &[Sample], value: impl Fn(&Sample) -> f32, minimum: f32) -> f32 {
    let peak = samples.iter().map(value).fold(0.0f32, f32::max);
    (peak * 1.2).max(minimum)
}

/// Gut sichtbarer Pfeil neben der Spalte, nach der sortiert wird.
fn sort_arrow(ui: &Ui, center: Pos2, color: Color32) {
    let (half_width, half_height) = (6.0, 4.0);
    ui.painter().add(Shape::convex_polygon(
        vec![
            Pos2::new(center.x - half_width, center.y - half_height),
            Pos2::new(center.x + half_width, center.y - half_height),
            Pos2::new(center.x, center.y + half_height),
        ],
        color,
        Stroke::NONE,
    ));
}

// ───────── Geräte (Leistung und Verlauf) ─────────

const DEVICE_LIST_WIDTH: f32 = 240.0;

/// Stand der Hardware für die Seiten „Leistung“ und „Verlauf“.
struct Hardware {
    cpu_name: String,
    cpu_cores: usize,
    cores: Vec<f32>,
    cpu_mhz: u64,
    uptime: u64,
    process_count: usize,
    total_ram: u64,
    used_ram: u64,
    swap_total: u64,
    swap_used: u64,
    gpus: Vec<GpuInfo>,
    disk_letters: Vec<String>,
    physical: Vec<PhysicalDisk>,
    networks: Vec<NetworkRow>,
    power: PowerBreakdown,
}

impl Hardware {
    fn from_state(state: &State) -> Self {
        Self {
            cpu_name: state.cpu_name.clone(),
            cpu_cores: state.cpu_cores,
            cores: state.cores.clone(),
            cpu_mhz: state.cpu_mhz,
            uptime: state.uptime,
            process_count: state.process_count,
            total_ram: state.total_ram,
            used_ram: state.used_ram,
            swap_total: state.swap_total,
            swap_used: state.swap_used,
            gpus: state.gpus.clone(),
            disk_letters: state.disk_letters.clone(),
            physical: state.physical_disks.clone(),
            networks: state.networks.clone(),
            power: state.power.clone(),
        }
    }

    /// Reihenfolge wie im Task-Manager: CPU, Arbeitsspeicher, Datenträger, Netzwerk, GPUs.
    fn devices(&self, last: &Sample) -> Vec<Device> {
        let mut devices = vec![Device::Cpu, Device::Memory];
        let disks = self.disk_letters.len().max(last.disks.len());
        devices.extend((0..disks).map(Device::Disk));
        devices.push(Device::Network);
        let gpus = self.gpus.len().max(usize::from(last.gpu.is_some()));
        devices.extend((0..gpus).map(Device::Gpu));
        devices
    }

    fn title(&self, device: Device) -> String {
        match device {
            Device::Cpu => "CPU".to_owned(),
            Device::Memory => "Arbeitsspeicher".to_owned(),
            Device::Gpu(index) => format!("GPU {index}"),
            Device::Disk(index) => match self.disk_letters.get(index) {
                Some(letters) if !letters.is_empty() => {
                    crate::i18n::td(format!("Datenträger {index} ({letters})"))
                }
                _ => crate::i18n::td(format!("Datenträger {index}")),
            },
            Device::Network => "Netzwerk".to_owned(),
        }
    }

    fn physical_disk(&self, index: usize) -> Option<&PhysicalDisk> {
        self.physical
            .iter()
            .find(|disk| disk.number as usize == index)
    }

    /// Modellbezeichnung des Geräts (rechts über dem Diagramm).
    fn model(&self, device: Device) -> String {
        match device {
            Device::Cpu => self.cpu_name.clone(),
            Device::Memory => format_bytes(self.total_ram),
            Device::Gpu(index) => self
                .gpus
                .get(index)
                .map(|gpu| gpu.name.clone())
                .unwrap_or_default(),
            Device::Disk(index) => self
                .physical_disk(index)
                .map(|disk| disk.model.clone())
                .unwrap_or_default(),
            Device::Network => self
                .networks
                .first()
                .map(|network| network.name.clone())
                .unwrap_or_default(),
        }
    }

    /// Kurzer aktueller Wert für die Geräteliste.
    fn summary(&self, device: Device, last: &Sample) -> String {
        match device {
            Device::Cpu => match self.cpu_mhz {
                0 => format!("{:.0} %", last.cpu),
                mhz => {
                    format!("{:.0} %  {:.2} GHz", last.cpu, mhz as f32 / 1000.0).replace('.', ",")
                }
            },
            Device::Memory => format!(
                "{} / {} ({:.0} %)",
                format_bytes(self.used_ram),
                format_bytes(self.total_ram),
                last.ram
            ),
            Device::Gpu(index) => match (gpu_value(last, index, 0), gpu_value(last, index, 1)) {
                (Some(usage), Some(temp)) => format!("{usage:.0} % ({temp:.0} °C)"),
                (Some(usage), None) => format!("{usage:.0} %"),
                _ => "–".to_owned(),
            },
            Device::Disk(index) => {
                let kind = self
                    .physical_disk(index)
                    .map(|disk| disk.bus.as_str())
                    .filter(|bus| !bus.is_empty())
                    .unwrap_or(t("Datenträger"));
                format!("{kind}  {:.0} %", disk_value(last, index, 2).unwrap_or(0.0))
            }
            Device::Network => format!(
                "↓ {}  ↑ {}",
                format_rate(last.net_down),
                format_rate(last.net_up)
            ),
        }
    }
}

/// Wert einer Grafikkarte: Spalte 0 = Auslastung, 1 = Temperatur. Ältere Messpunkte
/// kennen nur eine Karte – sie gelten für GPU 0.
fn gpu_value(sample: &Sample, index: usize, column: usize) -> Option<f32> {
    match sample.gpus.get(index) {
        Some(row) => Some(row[column]).filter(|value| column == 0 || *value > 0.0),
        None if index == 0 && sample.gpus.is_empty() => {
            if column == 0 {
                sample.gpu
            } else {
                sample.gpu_temp
            }
        }
        None => None,
    }
}

/// Wert eines Datenträgers: Spalte 0 = Lesen, 1 = Schreiben (Bytes/s), 2 = aktive Zeit (%).
fn disk_value(sample: &Sample, index: usize, column: usize) -> Option<f32> {
    sample.disks.get(index).map(|row| row[column])
}

fn device_color(p: &Palette, device: Device) -> Color32 {
    let (first, second, third) = series_colors(p);
    match device {
        Device::Cpu | Device::Disk(_) => first,
        Device::Memory | Device::Network => second,
        Device::Gpu(_) => third,
    }
}

/// Auslastung eines Geräts in Prozent für den kleinen Verlauf der Geräteliste.
fn device_load(device: Device, net_max: f32) -> Box<dyn Fn(&Sample) -> Option<f32>> {
    match device {
        Device::Cpu => Box::new(|s| Some(s.cpu)),
        Device::Memory => Box::new(|s| Some(s.ram)),
        Device::Gpu(index) => Box::new(move |s| gpu_value(s, index, 0)),
        Device::Disk(index) => Box::new(move |s| disk_value(s, index, 2)),
        Device::Network => Box::new(move |s| {
            Some((s.net_down + s.net_up) / BYTES_PER_MB / net_max.max(0.001) * 100.0)
        }),
    }
}

/// Ein Diagramm in einer Karte mit Überschrift.
fn chart_card(
    ui: &mut Ui,
    p: &Palette,
    title: &str,
    samples: &[Sample],
    lines: &[Line],
    max: f32,
    unit: &str,
    range: Range,
    now: u64,
) {
    theme::card(p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        theme::card_title(ui, title);
        chart(ui, p, samples, lines, max, unit, range, now);
    });
}

/// Die Diagramme eines Geräts. `with_power` ergänzt die Leistungsaufnahme (Verlauf).
fn device_charts(
    ui: &mut Ui,
    p: &Palette,
    device: Device,
    samples: &[Sample],
    range: Range,
    now: u64,
    with_power: bool,
) {
    let (first, second, third) = series_colors(p);
    let watts = |value: fn(&Sample) -> f32| (headroom(samples, value, 10.0) / 10.0).ceil() * 10.0;
    match device {
        Device::Cpu => {
            let lines = [Line {
                label: "Auslastung",
                color: first,
                value: &|s| Some(s.cpu),
            }];
            chart_card(ui, p, "Auslastung", samples, &lines, 100.0, "%", range, now);
            if samples.iter().any(|s| s.cpu_temp.is_some()) {
                let lines = [Line {
                    label: "Temperatur",
                    color: second,
                    value: &|s| s.cpu_temp,
                }];
                chart_card(
                    ui,
                    p,
                    "Temperatur",
                    samples,
                    &lines,
                    100.0,
                    "°C",
                    range,
                    now,
                );
            }
            if with_power {
                let lines = [Line {
                    label: "Leistung",
                    color: third,
                    value: &|s| Some(s.cpu_w),
                }];
                let max = watts(|s| s.cpu_w);
                chart_card(
                    ui,
                    p,
                    "Leistungsaufnahme",
                    samples,
                    &lines,
                    max,
                    "W",
                    range,
                    now,
                );
            }
        }
        Device::Memory => {
            let lines = [Line {
                label: "Belegt",
                color: second,
                value: &|s| Some(s.ram),
            }];
            chart_card(
                ui,
                p,
                "Speichernutzung",
                samples,
                &lines,
                100.0,
                "%",
                range,
                now,
            );
        }
        Device::Gpu(index) => {
            let lines = [Line {
                label: "Auslastung",
                color: third,
                value: &|s| gpu_value(s, index, 0),
            }];
            chart_card(ui, p, "Auslastung", samples, &lines, 100.0, "%", range, now);
            if samples.iter().any(|s| gpu_value(s, index, 1).is_some()) {
                let lines = [Line {
                    label: "Temperatur",
                    color: second,
                    value: &|s| gpu_value(s, index, 1),
                }];
                chart_card(
                    ui,
                    p,
                    "Temperatur",
                    samples,
                    &lines,
                    100.0,
                    "°C",
                    range,
                    now,
                );
            }
            if with_power {
                let lines = [Line {
                    label: "Leistung aller Grafikkarten",
                    color: first,
                    value: &|s| Some(s.gpu_w),
                }];
                let max = watts(|s| s.gpu_w);
                chart_card(
                    ui,
                    p,
                    "Leistungsaufnahme",
                    samples,
                    &lines,
                    max,
                    "W",
                    range,
                    now,
                );
            }
        }
        Device::Disk(index) => {
            let lines = [Line {
                label: "Aktive Zeit",
                color: first,
                value: &|s| disk_value(s, index, 2),
            }];
            chart_card(
                ui,
                p,
                "Aktive Zeit",
                samples,
                &lines,
                100.0,
                "%",
                range,
                now,
            );
            let lines = [
                Line {
                    label: "Lesen",
                    color: third,
                    value: &|s| disk_value(s, index, 0).map(|bytes| bytes / BYTES_PER_MB),
                },
                Line {
                    label: "Schreiben",
                    color: second,
                    value: &|s| disk_value(s, index, 1).map(|bytes| bytes / BYTES_PER_MB),
                },
            ];
            let max = headroom(
                samples,
                |s| {
                    let read = disk_value(s, index, 0).unwrap_or(0.0);
                    read.max(disk_value(s, index, 1).unwrap_or(0.0)) / BYTES_PER_MB
                },
                1.0,
            );
            chart_card(
                ui,
                p,
                "Übertragungsrate",
                samples,
                &lines,
                max,
                "MB/s",
                range,
                now,
            );
        }
        Device::Network => {
            let lines = [
                Line {
                    label: "Empfangen",
                    color: third,
                    value: &|s| Some(s.net_down / BYTES_PER_MB),
                },
                Line {
                    label: "Gesendet",
                    color: second,
                    value: &|s| Some(s.net_up / BYTES_PER_MB),
                },
            ];
            let max = headroom(samples, |s| s.net_down.max(s.net_up) / BYTES_PER_MB, 0.5);
            chart_card(ui, p, "Durchsatz", samples, &lines, max, "MB/s", range, now);
        }
    }
}

/// Betriebszeit wie im Task-Manager: Tage:Stunden:Minuten:Sekunden.
fn format_uptime(seconds: u64) -> String {
    format!(
        "{}:{:02}:{:02}:{:02}",
        seconds / 86_400,
        seconds % 86_400 / 3600,
        seconds % 3600 / 60,
        seconds % 60
    )
}

fn measured_label(measured: bool) -> &'static str {
    if measured {
        "gemessen"
    } else {
        "geschätzt"
    }
}

/// Kennzahlen des Geräts unter den Diagrammen.
fn device_facts(ui: &mut Ui, p: &Palette, device: Device, hardware: &Hardware, last: &Sample) {
    let unknown = || "nicht verfügbar".to_owned();
    match device {
        Device::Cpu => {
            ui.horizontal_wrapped(|ui| {
                detail(ui, p, t("Auslastung"), format!("{:.0} %", last.cpu));
                if hardware.cpu_mhz > 0 {
                    let speed = format!("{:.2} GHz", hardware.cpu_mhz as f32 / 1000.0);
                    detail(ui, p, t("Geschwindigkeit"), speed.replace('.', ","));
                }
                detail(ui, p, t("Prozesse"), hardware.process_count.to_string());
                detail(
                    ui,
                    p,
                    t("Logische Prozessoren"),
                    hardware.cpu_cores.to_string(),
                );
                detail(ui, p, t("Betriebszeit"), format_uptime(hardware.uptime));
                let temperature = last.cpu_temp.map_or_else(unknown, |t| format!("{t:.0} °C"));
                detail(ui, p, t("Temperatur"), temperature);
                let power = format!(
                    "{:.0} W ({})",
                    hardware.power.cpu_w,
                    measured_label(hardware.power.cpu_measured)
                );
                detail(ui, p, t("Leistung"), power);
            });

            // Auslastung je logischem Prozessor als Balkenreihe.
            if !hardware.cores.is_empty() {
                ui.add_space(6.0);
                ui.label(theme::secondary(p, t("Logische Prozessoren")).size(12.0));
                let (cell, gap) = (egui::vec2(26.0, 44.0), 4.0);
                let per_row = ((ui.available_width() + gap) / (cell.x + gap))
                    .floor()
                    .max(1.0) as usize;
                let rows = hardware.cores.len().div_ceil(per_row);
                let size = egui::vec2(ui.available_width(), rows as f32 * (cell.y + gap));
                let (area, _) = ui.allocate_exact_size(size, Sense::hover());
                for (index, usage) in hardware.cores.iter().enumerate() {
                    let offset = egui::vec2(
                        (index % per_row) as f32 * (cell.x + gap),
                        (index / per_row) as f32 * (cell.y + gap),
                    );
                    let rect = Rect::from_min_size(area.min + offset, cell);
                    ui.painter().rect_filled(rect, 2.0, p.field_bg);
                    let mut fill = rect;
                    fill.set_top(rect.bottom() - rect.height() * (usage / 100.0).clamp(0.0, 1.0));
                    ui.painter()
                        .rect_filled(fill, 2.0, p.accent.gamma_multiply(0.75));
                    ui.painter()
                        .rect_stroke(rect, 2.0, Stroke::new(1.0_f32, p.separator));
                    ui.painter().text(
                        Pos2::new(rect.center().x, rect.bottom() - 2.0),
                        Align2::CENTER_BOTTOM,
                        format!("{usage:.0}"),
                        FontId::proportional(10.5),
                        p.text,
                    );
                }
            }
        }
        Device::Memory => {
            ui.horizontal_wrapped(|ui| {
                detail(ui, p, t("In Verwendung"), format_bytes(hardware.used_ram));
                let free = hardware.total_ram.saturating_sub(hardware.used_ram);
                detail(ui, p, t("Verfügbar"), format_bytes(free));
                detail(ui, p, t("Gesamt"), format_bytes(hardware.total_ram));
                if hardware.swap_total > 0 {
                    let swap = format!(
                        "{} / {}",
                        format_bytes(hardware.swap_used),
                        format_bytes(hardware.swap_total)
                    );
                    detail(ui, p, t("Auslagerungsdatei"), swap);
                }
                detail(
                    ui,
                    p,
                    t("Leistung (geschätzt)"),
                    format!("{:.0} W", hardware.power.ram_w),
                );
            });
        }
        Device::Gpu(index) => {
            ui.horizontal_wrapped(|ui| {
                let usage = gpu_value(last, index, 0)
                    .map_or_else(|| "–".to_owned(), |u| format!("{u:.0} %"));
                detail(ui, p, t("Auslastung"), usage);
                let temperature =
                    gpu_value(last, index, 1).map_or_else(unknown, |t| format!("{t:.0} °C"));
                detail(ui, p, t("Temperatur"), temperature);
                if let Some(memory) = hardware.gpus.get(index).and_then(|gpu| gpu.memory_used) {
                    detail(
                        ui,
                        p,
                        t("Dedizierter Speicher belegt"),
                        format_bytes(memory),
                    );
                }
                let power = format!(
                    "{:.0} W ({})",
                    hardware.power.gpu_w,
                    measured_label(hardware.power.gpu_measured)
                );
                detail(ui, p, t("Leistung aller Grafikkarten"), power);
            });
        }
        Device::Disk(index) => {
            ui.horizontal_wrapped(|ui| {
                let active = disk_value(last, index, 2).unwrap_or(0.0);
                detail(ui, p, t("Aktive Zeit"), format!("{active:.0} %"));
                detail(
                    ui,
                    p,
                    t("Lesen"),
                    format_rate(disk_value(last, index, 0).unwrap_or(0.0)),
                );
                detail(
                    ui,
                    p,
                    t("Schreiben"),
                    format_rate(disk_value(last, index, 1).unwrap_or(0.0)),
                );
                if let Some(disk) = hardware.physical_disk(index) {
                    detail(ui, p, t("Kapazität"), format_bytes(disk.size));
                    if !disk.bus.is_empty() {
                        detail(ui, p, t("Anschluss"), disk.bus.clone());
                    }
                    if !disk.style.is_empty() {
                        detail(ui, p, t("Partitionsstil"), disk.style.clone());
                    }
                    detail(ui, p, t("Partitionen"), disk.partitions.len().to_string());
                }
            });
        }
        Device::Network => {
            ui.horizontal_wrapped(|ui| {
                detail(ui, p, t("Empfangen"), format_rate(last.net_down));
                detail(ui, p, t("Gesendet"), format_rate(last.net_up));
            });
            if !hardware.networks.is_empty() {
                ui.add_space(6.0);
                egui::Grid::new("monitor-networks")
                    .num_columns(3)
                    .striped(true)
                    .spacing(egui::vec2(24.0, 5.0))
                    .show(ui, |ui| {
                        for title in ["Adapter", "Empfangen", "Gesendet"] {
                            ui.label(theme::secondary(p, title).size(12.5));
                        }
                        ui.end_row();
                        for network in &hardware.networks {
                            ui.label(network.name.as_str());
                            ui.label(format_rate(network.down));
                            ui.label(format_rate(network.up));
                            ui.end_row();
                        }
                    });
            }
        }
    }
}

impl MonitorView {
    pub fn ui(&mut self, ui: &mut Ui, shared: &mut Shared) {
        if let Some(message) = shared.monitor.take_message() {
            shared.notify(message);
        }
        self.receive_icons(ui);
        let p = shared.palette;

        ui.horizontal(|ui| {
            for (page, label) in Page::ALL {
                if theme::chip(ui, &p, label, self.page == page).clicked() {
                    self.page = page;
                }
            }
        });
        ui.add_space(4.0);

        match self.page {
            Page::Processes => self.processes(ui, shared),
            Page::Performance => self.performance(ui, shared),
            Page::History => self.history(ui, shared),
            Page::Disks => self.disks(ui, shared),
            Page::Energy => self.energy(ui, shared),
        }
    }

    fn range_chips(&mut self, ui: &mut Ui, p: &Palette) {
        ui.horizontal(|ui| {
            ui.label(theme::secondary(p, t("Zeitraum")).size(12.5));
            for range in Range::ALL {
                if theme::chip(ui, p, range.label(), self.range == range).clicked() {
                    self.range = range;
                }
            }
        });
    }

    // ───────── Icons ─────────

    /// Übernimmt fertig geladene Icons als Texturen.
    fn receive_icons(&mut self, ui: &Ui) {
        let Some(loader) = &self.icon_loader else {
            return;
        };
        for (path, icon) in loader.results.try_iter() {
            let texture = icon.map(|icon| {
                let image =
                    egui::ColorImage::from_rgba_unmultiplied([icon.width, icon.height], &icon.rgba);
                ui.ctx().load_texture(
                    format!("process-icon:{path}"),
                    image,
                    egui::TextureOptions::LINEAR,
                )
            });
            self.icons.insert(path, texture);
        }
    }

    /// Fordert Icons für Programme an, die noch keines haben.
    fn request_icons<'a>(&mut self, ui: &Ui, rows: impl Iterator<Item = &'a ProcessRow>) {
        for row in rows {
            let Some(exe) = &row.exe else {
                continue;
            };
            if self.icons.contains_key(exe) {
                continue;
            }
            let loader = self
                .icon_loader
                .get_or_insert_with(|| IconLoader::spawn(ui.ctx().clone()));
            let _ = loader.requests.send(exe.clone());
            self.icons.insert(exe.clone(), None);
        }
    }

    fn icon_of(&self, row: &ProcessRow) -> Option<&TextureHandle> {
        self.icons.get(row.exe.as_ref()?)?.as_ref()
    }

    // ───────── Prozesse ─────────

    fn processes(&mut self, ui: &mut Ui, shared: &mut Shared) {
        let p = shared.palette;
        let (mut rows, cpu_total, ram_total, disk_total): (Vec<ProcessRow>, f32, f32, f32) =
            shared.monitor.read(|state| {
                let last = state.recent.back();
                (
                    state.processes.clone(),
                    last.map_or(0.0, |s| s.cpu),
                    last.map_or(0.0, |s| s.ram),
                    last.map_or(0.0, |s| s.disk_read + s.disk_write),
                )
            });
        let total = rows.len();

        let needle = self.search.trim().to_lowercase();
        if !needle.is_empty() {
            rows.retain(|row| row.name.to_lowercase().contains(&needle));
        }
        match self.sort {
            Sort::Cpu => rows.sort_by(|a, b| b.cpu.total_cmp(&a.cpu)),
            Sort::Memory => rows.sort_by(|a, b| b.memory.cmp(&a.memory)),
            Sort::Disk => rows.sort_by(|a, b| {
                (b.disk_read + b.disk_write).total_cmp(&(a.disk_read + a.disk_write))
            }),
            Sort::Name => rows.sort_by_key(|row| row.name.to_lowercase()),
        }
        rows.truncate(MAX_PROCESS_ROWS);
        self.request_icons(ui, rows.iter());

        // Die Markierung verfällt, wenn das Programm nicht mehr läuft.
        let selected_row = self
            .selected
            .as_ref()
            .and_then(|name| rows.iter().find(|row| &row.name == name))
            .cloned();
        if selected_row.is_none() {
            self.selected = None;
        }

        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = RichText::new(t("Task beenden")).color(p.danger);
                if ui
                    .add_enabled(selected_row.is_some(), egui::Button::new(label))
                    .on_hover_text(t("Beendet alle Prozesse des markierten Programms"))
                    .clicked()
                {
                    if let Some(row) = &selected_row {
                        self.confirm_kill = Some((row.name.clone(), row.pids.clone()));
                    }
                }
                ui.label(
                    theme::secondary(&p, crate::i18n::td(format!("{total} Programme"))).size(12.5),
                );
                ui.add(
                    TextEdit::singleline(&mut self.search)
                        .hint_text(t("Programme durchsuchen"))
                        .desired_width(f32::INFINITY),
                );
            });
        });

        // Sicherheitsabfrage vor dem Beenden.
        if let Some((name, pids)) = self.confirm_kill.clone() {
            theme::card(&p).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(crate::i18n::td(format!(
                        "„{name}“ beenden ({} {})? Nicht gespeicherte Daten gehen verloren.",
                        pids.len(),
                        if pids.len() == 1 {
                            "Prozess"
                        } else {
                            "Prozesse"
                        }
                    )));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button(t("Abbrechen")).clicked() {
                            self.confirm_kill = None;
                        }
                        if ui
                            .button(RichText::new(t("Beenden")).color(p.danger))
                            .clicked()
                        {
                            shared
                                .monitor
                                .send(Command::Kill(name.clone(), pids.clone()));
                            self.confirm_kill = None;
                            self.selected = None;
                        }
                    });
                });
            });
        }

        let mut open_error = None;
        theme::card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 1.0;

            // Spaltenköpfe wie im Task-Manager: oben die Gesamtauslastung, darunter der Name.
            let (header, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), Sense::hover());
            let disk_right = header.right() - 12.0;
            let memory_right = disk_right - 110.0;
            let cpu_right = memory_right - 120.0;
            let name_right = cpu_right - 80.0;
            let columns = [
                ("CPU", format!("{cpu_total:.0} %"), Sort::Cpu, cpu_right),
                (
                    "Arbeitsspeicher",
                    format!("{ram_total:.0} %"),
                    Sort::Memory,
                    memory_right,
                ),
                (
                    "Datenträger",
                    format_rate(disk_total),
                    Sort::Disk,
                    disk_right,
                ),
            ];
            for (label, value, sort, right) in columns {
                let rect = Rect::from_min_max(
                    Pos2::new(right - 105.0, header.top()),
                    Pos2::new(right + 4.0, header.bottom()),
                );
                let response = ui.interact(
                    rect,
                    ui.id().with(("process-column", label)),
                    Sense::click(),
                );
                let active = self.sort == sort;
                let color = if active { p.text } else { p.text_secondary };
                ui.painter().text(
                    Pos2::new(right, header.top() + 2.0),
                    Align2::RIGHT_TOP,
                    value,
                    FontId::proportional(15.0),
                    p.text,
                );
                let label_rect = ui.painter().text(
                    Pos2::new(right, header.bottom() - 3.0),
                    Align2::RIGHT_BOTTOM,
                    label,
                    FontId::proportional(12.0),
                    color,
                );
                if active {
                    sort_arrow(
                        ui,
                        Pos2::new(label_rect.left() - 11.0, label_rect.center().y),
                        p.text,
                    );
                }
                if response.clicked() {
                    self.sort = sort;
                }
            }
            let name_rect =
                Rect::from_min_max(header.left_top(), Pos2::new(name_right, header.bottom()));
            let name_response = ui.interact(
                name_rect,
                ui.id().with("process-column-name"),
                Sense::click(),
            );
            let name_label = ui.painter().text(
                Pos2::new(header.left() + 10.0, header.bottom() - 3.0),
                Align2::LEFT_BOTTOM,
                t("Name"),
                FontId::proportional(12.0),
                if self.sort == Sort::Name {
                    p.text
                } else {
                    p.text_secondary
                },
            );
            if self.sort == Sort::Name {
                sort_arrow(
                    ui,
                    Pos2::new(name_label.right() + 11.0, name_label.center().y),
                    p.text,
                );
            }
            if name_response.clicked() {
                self.sort = Sort::Name;
            }
            ui.separator();

            // Zwei Gruppen wie im Task-Manager: Programme mit Fenster, dann der Rest.
            let font = FontId::proportional(13.5);
            for (title, is_app) in [("Apps", true), ("Hintergrundprozesse", false)] {
                let group: Vec<&ProcessRow> =
                    rows.iter().filter(|row| row.is_app == is_app).collect();
                if group.is_empty() {
                    continue;
                }
                ui.add_space(6.0);
                ui.label(RichText::new(format!("  {title} ({})", group.len())).size(14.5));
                ui.add_space(2.0);

                for row in group {
                    let size = egui::vec2(ui.available_width(), PROCESS_ROW_HEIGHT);
                    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
                    let selected = self.selected.as_deref() == Some(row.name.as_str());
                    if ui.is_rect_visible(rect) {
                        if selected {
                            ui.painter().rect_filled(rect, p.radius, p.selection);
                        } else if response.hovered() {
                            ui.painter().rect_filled(
                                rect,
                                p.radius,
                                p.selection.gamma_multiply(0.6),
                            );
                        }

                        // Icon (oder Platzhalter) und Name.
                        let icon_rect = Rect::from_center_size(
                            Pos2::new(rect.left() + 20.0, rect.center().y),
                            egui::vec2(16.0, 16.0),
                        );
                        match self.icon_of(row) {
                            Some(texture) => {
                                let uv =
                                    Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(1.0, 1.0));
                                ui.painter()
                                    .image(texture.id(), icon_rect, uv, Color32::WHITE);
                            }
                            None => {
                                ui.painter().rect_filled(
                                    icon_rect.shrink(2.0),
                                    2.0,
                                    p.control_hover,
                                );
                            }
                        }
                        let name = if row.pids.len() > 1 {
                            format!("{} ({})", display_name(&row.name), row.pids.len())
                        } else {
                            display_name(&row.name).to_owned()
                        };
                        let name_clip =
                            Rect::from_min_max(rect.min, Pos2::new(name_right, rect.max.y));
                        ui.painter().with_clip_rect(name_clip).text(
                            Pos2::new(rect.left() + 36.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            name,
                            font.clone(),
                            p.text,
                        );

                        // Zellen werden mit steigender Last kräftiger hinterlegt.
                        let disk = row.disk_read + row.disk_write;
                        let cells = [
                            (cpu_right, format_percent(row.cpu), row.cpu / 50.0),
                            (
                                memory_right,
                                format_bytes(row.memory),
                                row.memory as f32 / 4.0e9,
                            ),
                            (disk_right, format_rate(disk), disk / (50.0 * BYTES_PER_MB)),
                        ];
                        for (right, text, heat) in cells {
                            let cell = Rect::from_min_max(
                                Pos2::new(right - 100.0, rect.top() + 1.0),
                                Pos2::new(right + 6.0, rect.bottom() - 1.0),
                            );
                            let strength = 0.08 + 0.5 * heat.clamp(0.0, 1.0);
                            ui.painter()
                                .rect_filled(cell, 2.0, p.accent.gamma_multiply(strength));
                            ui.painter().text(
                                Pos2::new(right, rect.center().y),
                                Align2::RIGHT_CENTER,
                                text,
                                font.clone(),
                                p.text,
                            );
                        }
                    }
                    if response.clicked() {
                        self.selected = Some(row.name.clone());
                    }
                    response.context_menu(|ui| {
                        if ui
                            .button(RichText::new(t("Task beenden")).color(p.danger))
                            .clicked()
                        {
                            self.confirm_kill = Some((row.name.clone(), row.pids.clone()));
                            ui.close_menu();
                        }
                        if let Some(folder) =
                            row.exe.as_deref().and_then(|exe| Path::new(exe).parent())
                        {
                            if ui.button(t("Dateipfad öffnen")).clicked() {
                                open_error = platform::open_folder(folder).err();
                                ui.close_menu();
                            }
                        }
                    });
                }
            }
            if rows.is_empty() {
                ui.label(theme::secondary(&p, t("Keine Programme gefunden.")));
            }
        });
        if let Some(err) = open_error {
            shared.notify(crate::i18n::td(format!("Dateisystem: {err}")));
        }
    }

    // ───────── Leistung (wie im Task-Manager) ─────────

    fn performance(&mut self, ui: &mut Ui, shared: &mut Shared) {
        let p = shared.palette;
        let now = unix_now();
        let (samples, hardware) = shared.monitor.read(|state| {
            (
                collect_samples(state, Range::Minute, now),
                Hardware::from_state(state),
            )
        });
        let last = samples.last().cloned().unwrap_or_default();
        let devices = hardware.devices(&last);
        if !devices.contains(&self.device) {
            self.device = Device::Cpu;
        }
        let net_max = headroom(&samples, |s| (s.net_down + s.net_up) / BYTES_PER_MB, 0.5);

        ui.horizontal_top(|ui| {
            // Linke Spalte: Geräte mit kleinem Verlauf.
            ui.vertical(|ui| {
                ui.set_width(DEVICE_LIST_WIDTH);
                ui.spacing_mut().item_spacing.y = 2.0;
                for &device in &devices {
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(DEVICE_LIST_WIDTH, 58.0), Sense::click());
                    if self.device == device {
                        ui.painter().rect_filled(rect, p.radius, p.selection);
                    } else if response.hovered() {
                        ui.painter()
                            .rect_filled(rect, p.radius, p.selection.gamma_multiply(0.6));
                    }
                    let graph = Rect::from_min_size(
                        rect.min + egui::vec2(8.0, 9.0),
                        egui::vec2(60.0, 40.0),
                    );
                    let value = device_load(device, net_max);
                    sparkline(
                        ui,
                        &p,
                        graph,
                        &samples,
                        &*value,
                        100.0,
                        device_color(&p, device),
                    );
                    let text = ui.painter().with_clip_rect(rect);
                    text.text(
                        Pos2::new(graph.right() + 10.0, rect.top() + 9.0),
                        Align2::LEFT_TOP,
                        hardware.title(device),
                        FontId::proportional(15.0),
                        p.text,
                    );
                    text.text(
                        Pos2::new(graph.right() + 10.0, rect.bottom() - 9.0),
                        Align2::LEFT_BOTTOM,
                        hardware.summary(device, &last),
                        FontId::proportional(12.0),
                        p.text_secondary,
                    );
                    if response.clicked() {
                        self.device = device;
                    }
                }
            });

            // Rechte Spalte: Diagramme und Kennzahlen des gewählten Geräts.
            ui.vertical(|ui| {
                let device = self.device;
                ui.horizontal(|ui| {
                    ui.label(RichText::new(hardware.title(device)).size(24.0));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(theme::secondary(&p, hardware.model(device)))
                                .truncate(),
                        );
                    });
                });
                device_charts(ui, &p, device, &samples, Range::Minute, now, false);
                ui.add_space(4.0);
                device_facts(ui, &p, device, &hardware, &last);
            });
        });
    }

    // ───────── Verlauf je Komponente ─────────

    fn history(&mut self, ui: &mut Ui, shared: &mut Shared) {
        let p = shared.palette;
        let now = unix_now();
        let range = self.range;
        let span = range.overview_seconds();
        // Das Fenster darf höchstens bis zum Anfang der Zeitleiste zurückreichen.
        self.history_offset = self
            .history_offset
            .min(span.saturating_sub(range.seconds()));
        let end = now.saturating_sub(self.history_offset);
        let start = end.saturating_sub(range.seconds());
        let (samples, overview, hardware) = shared.monitor.read(|state| {
            (
                collect_between(state, start, end, range.uses_minutes()),
                collect_between(state, now.saturating_sub(span), now, span > 3600),
                Hardware::from_state(state),
            )
        });
        let last = shared
            .monitor
            .read(|state| state.recent.back().cloned())
            .unwrap_or_default();
        let devices = hardware.devices(&last);
        if !devices.contains(&self.history_device) {
            self.history_device = Device::Cpu;
        }

        ui.horizontal_wrapped(|ui| {
            for &device in &devices {
                let title = hardware.title(device);
                if theme::chip(ui, &p, &title, self.history_device == device)
                    .on_hover_text(hardware.model(device))
                    .clicked()
                {
                    self.history_device = device;
                }
            }
            ui.add_space(10.0);
            egui::ComboBox::from_id_salt("monitor-history-range")
                .selected_text(self.range.label())
                .show_ui(ui, |ui| {
                    for option in Range::ALL {
                        ui.selectable_value(&mut self.range, option, option.label());
                    }
                });
        });

        device_charts(ui, &p, self.history_device, &samples, range, end, true);

        // ── Zeitleiste: größerer Zeitraum, das gezeigte Fenster lässt sich verschieben ──
        theme::card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let step = (range.seconds() / 2).max(1);
                if ui.button("◀").on_hover_text(t("Früher")).clicked() {
                    self.history_offset += step;
                }
                if ui
                    .add_enabled(self.history_offset > 0, egui::Button::new("▶"))
                    .on_hover_text(t("Später"))
                    .clicked()
                {
                    self.history_offset = self.history_offset.saturating_sub(step);
                }
                if ui
                    .add_enabled(self.history_offset > 0, egui::Button::new(t("Jetzt")))
                    .clicked()
                {
                    self.history_offset = 0;
                }
                let with_date = span > 86_400;
                let window = if self.history_offset == 0 {
                    crate::i18n::td(format!("Live · letzte {}", range.label()))
                } else {
                    format!(
                        "{} – {}",
                        format_clock(start, with_date),
                        format_clock(end, with_date)
                    )
                };
                ui.label(theme::secondary(&p, window).size(12.5));
            });

            let size = egui::vec2(ui.available_width(), TIMELINE_HEIGHT);
            let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, p.radius, p.field_bg);
            let plot = rect.shrink2(egui::vec2(6.0, 4.0));
            let first = now.saturating_sub(span);
            let x_of = |time: u64| {
                plot.left()
                    + (time.saturating_sub(first) as f32 / span as f32).clamp(0.0, 1.0)
                        * plot.width()
            };

            // Verlauf der gewählten Komponente über den ganzen Zeitraum.
            let net_max = headroom(&overview, |s| (s.net_down + s.net_up) / BYTES_PER_MB, 0.5);
            let value = device_load(self.history_device, net_max);
            let color = device_color(&p, self.history_device);
            let max_gap = (span / 60).max(if span > 3600 { 180 } else { 25 });
            let mut segment: Vec<Pos2> = Vec::new();
            let mut previous = 0u64;
            let flush = |segment: &mut Vec<Pos2>| {
                if segment.len() > 1 {
                    fill_below(&painter, segment, plot.bottom(), color.gamma_multiply(0.18));
                    painter.add(Shape::line(
                        std::mem::take(segment),
                        Stroke::new(1.0_f32, color.gamma_multiply(0.8)),
                    ));
                } else {
                    segment.clear();
                }
            };
            for sample in &overview {
                let Some(load) = value(sample) else {
                    flush(&mut segment);
                    continue;
                };
                if !segment.is_empty() && sample.time.saturating_sub(previous) > max_gap {
                    flush(&mut segment);
                }
                let y = plot.bottom() - (load / 100.0).clamp(0.0, 1.0) * plot.height();
                segment.push(Pos2::new(x_of(sample.time), y));
                previous = sample.time;
            }
            flush(&mut segment);

            // Gezeigtes Fenster hervorheben.
            let selection = Rect::from_min_max(
                Pos2::new(x_of(start), rect.top() + 1.0),
                Pos2::new(x_of(end).max(x_of(start) + 4.0), rect.bottom() - 1.0),
            );
            painter.rect_filled(selection, 3.0, p.accent.gamma_multiply(0.14));
            painter.rect_stroke(selection, 3.0, Stroke::new(1.5_f32, p.accent));

            let font = FontId::proportional(10.5);
            painter.text(
                Pos2::new(plot.left(), rect.bottom() - 1.0),
                Align2::LEFT_BOTTOM,
                format_clock(first, span > 86_400),
                font.clone(),
                p.text_secondary,
            );
            painter.text(
                Pos2::new(plot.right(), rect.bottom() - 1.0),
                Align2::RIGHT_BOTTOM,
                format_clock(now, span > 86_400),
                font,
                p.text_secondary,
            );

            // Klicken oder Ziehen setzt die Mitte des Fensters auf den Mauszeiger.
            let pointer = response.interact_pointer_pos();
            if let Some(pointer) = pointer.filter(|_| response.clicked() || response.dragged()) {
                let share = ((pointer.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
                let center = first + (share as f64 * span as f64) as u64;
                let wanted_end = (center + range.seconds() / 2).min(now);
                let offset = now - wanted_end;
                // Nahe am rechten Rand rastet das Fenster wieder auf „live“ ein.
                self.history_offset = if offset < range.seconds() / 20 {
                    0
                } else {
                    offset
                };
            }
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
        });

        ui.label(
            theme::secondary(
                &p,
                "Zeige mit der Maus auf einen Zeitpunkt, um zu sehen, welches Programm dort die meiste Last erzeugte. \
                 In der Zeitleiste verschiebst du das Fenster per Klick oder Ziehen. Aufgezeichnet wird auch, \
                 während Toolless im Tray läuft; 7 Tage bleiben gespeichert.",
            )
            .size(12.0),
        );
    }

    // ───────── Datenträger (wie die Datenträgerverwaltung) ─────────

    fn disks(&mut self, ui: &mut Ui, shared: &mut Shared) {
        let p = shared.palette;
        let (volumes, physical): (Vec<DiskRow>, Vec<PhysicalDisk>) = shared
            .monitor
            .read(|state| (state.disks.clone(), state.physical_disks.clone()));
        let mut error = None;

        ui.horizontal(|ui| {
            if ui.button(t("Aktualisieren")).clicked() {
                shared.monitor.send(Command::RefreshDisks);
            }
            if cfg!(windows) {
                if ui
                    .button(t("Windows-Datenträgerverwaltung"))
                    .on_hover_text(t("Zum Ändern von Partitionen und Laufwerksbuchstaben"))
                    .clicked()
                {
                    error = platform::run_system_tool("diskmgmt.msc").err();
                }
                if ui.button(t("Datenträgerbereinigung")).clicked() {
                    error = platform::run_system_tool("cleanmgr.exe").err();
                }
            }
        });

        // Obere Hälfte: Liste der Volumes.
        theme::card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if volumes.is_empty() {
                ui.label(theme::secondary(&p, t("Datenträger werden gelesen …")));
                return;
            }
            egui::Grid::new("monitor-volumes")
                .num_columns(8)
                .striped(true)
                .spacing(egui::vec2(22.0, 6.0))
                .show(ui, |ui| {
                    for title in [
                        "Volume",
                        "Name",
                        "Dateisystem",
                        "Typ",
                        "Kapazität",
                        "Frei",
                        "% frei",
                        "",
                    ] {
                        ui.label(theme::secondary(&p, title).size(12.5));
                    }
                    ui.end_row();
                    for volume in &volumes {
                        let free_share = if volume.total > 0 {
                            volume.free as f32 / volume.total as f32 * 100.0
                        } else {
                            0.0
                        };
                        ui.label(RichText::new(volume.mount.trim_end_matches('\\')).strong());
                        ui.label(volume.name.as_str());
                        ui.label(volume.file_system.as_str());
                        let kind = if volume.removable {
                            "Wechseldatenträger"
                        } else {
                            volume.kind.as_str()
                        };
                        ui.label(if kind == "Unknown" { "–" } else { kind });
                        ui.label(format_bytes(volume.total));
                        ui.label(format_bytes(volume.free));
                        // Fast volle Laufwerke fallen farblich auf.
                        let share = RichText::new(format!("{free_share:.0} %"));
                        ui.label(if free_share < 10.0 {
                            share.color(p.danger)
                        } else {
                            share
                        });
                        if ui.small_button(t("Öffnen")).clicked() {
                            error = platform::open_folder(Path::new(&volume.mount)).err();
                        }
                        ui.end_row();
                    }
                });
        });

        // Untere Hälfte: physische Datenträger mit ihrer Partitionsaufteilung.
        for disk in &physical {
            theme::card(&p).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    ui.vertical(|ui| {
                        ui.set_width(170.0);
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.label(
                            RichText::new(crate::i18n::td(format!("Datenträger {}", disk.number)))
                                .strong(),
                        );
                        if !disk.model.is_empty() {
                            ui.label(theme::secondary(&p, disk.model.as_str()).size(12.0));
                        }
                        let mut facts = vec![format_bytes(disk.size), disk.style.clone()];
                        if !disk.bus.is_empty() {
                            facts.push(disk.bus.clone());
                        }
                        if disk.removable {
                            facts.push(t("Wechselmedium").to_owned());
                        }
                        ui.label(theme::secondary(&p, facts.join(" · ")).size(12.0));
                    });

                    // Partitionen proportional zu ihrer Größe, aber nie schmaler als lesbar.
                    let count = disk.partitions.len().max(1) as f32;
                    let available = ui.available_width() - 3.0 * count;
                    let minimum = 86.0f32.min(available / count).max(8.0);
                    let flexible = (available - minimum * count).max(0.0);
                    for partition in &disk.partitions {
                        let share = partition.size as f32 / disk.size.max(1) as f32;
                        let width = minimum + flexible * share;
                        let (rect, response) =
                            ui.allocate_exact_size(egui::vec2(width, 78.0), Sense::hover());
                        let volume = volumes.iter().find(|volume| {
                            !partition.letter.is_empty()
                                && volume.mount.starts_with(&partition.letter)
                        });

                        // Farbstreifen oben: belegte Partition in Akzentfarbe, freier Bereich dunkel.
                        let strip_color = if partition.unallocated {
                            p.text_secondary
                        } else {
                            p.accent
                        };
                        ui.painter().rect_filled(rect, 2.0, p.field_bg);
                        let strip = Rect::from_min_size(rect.min, egui::vec2(rect.width(), 7.0));
                        ui.painter().rect_filled(strip, 2.0, strip_color);
                        ui.painter()
                            .rect_stroke(rect, 2.0, Stroke::new(1.0_f32, p.separator));

                        let headline = if partition.unallocated {
                            "Nicht zugeordnet".to_owned()
                        } else if let Some(volume) = volume {
                            format!("{} {}", partition.letter, volume.name)
                                .trim()
                                .to_owned()
                        } else {
                            partition.kind.clone()
                        };
                        let detail_line = match volume {
                            Some(volume) => crate::i18n::td(format!(
                                "{} · {} frei",
                                volume.file_system,
                                format_bytes(volume.free)
                            )),
                            None if partition.unallocated => String::new(),
                            None => "ohne Laufwerksbuchstabe".to_owned(),
                        };
                        let clipped = ui.painter().with_clip_rect(rect.shrink(4.0));
                        let left = rect.left() + 7.0;
                        clipped.text(
                            Pos2::new(left, rect.top() + 12.0),
                            Align2::LEFT_TOP,
                            &headline,
                            FontId::proportional(13.0),
                            p.text,
                        );
                        clipped.text(
                            Pos2::new(left, rect.top() + 31.0),
                            Align2::LEFT_TOP,
                            format_bytes(partition.size),
                            FontId::proportional(12.0),
                            p.text_secondary,
                        );
                        clipped.text(
                            Pos2::new(left, rect.top() + 49.0),
                            Align2::LEFT_TOP,
                            &detail_line,
                            FontId::proportional(12.0),
                            p.text_secondary,
                        );
                        response.on_hover_text(format!(
                            "{headline}\n{}\n{}\n{detail_line}",
                            partition.kind,
                            format_bytes(partition.size)
                        ));
                    }
                });
            });
        }
        if physical.is_empty() && cfg!(windows) {
            ui.label(theme::secondary(
                &p,
                t("Der Aufbau der physischen Datenträger wird gelesen …"),
            ));
        }
        ui.label(
            theme::secondary(
                &p,
                t("Diese Ansicht ist nur lesend. Partitionen ändern, formatieren oder Laufwerksbuchstaben \
                 vergeben geht bewusst nur in der Windows-Datenträgerverwaltung."),
            )
            .size(12.0),
        );

        if let Some(err) = error {
            shared.notify(crate::i18n::td(format!("Dateisystem: {err}")));
        }
    }

    // ───────── Energie (angelehnt an WattSeal) ─────────

    fn energy(&mut self, ui: &mut Ui, shared: &mut Shared) {
        let p = shared.palette;
        let now = unix_now();
        let range = self.range;
        let (samples, power, mut rows): (Vec<Sample>, PowerBreakdown, Vec<ProcessRow>) =
            shared.monitor.read(|state| {
                (
                    collect_samples(state, range, now),
                    state.power.clone(),
                    state.processes.clone(),
                )
            });
        let last = samples.last().cloned().unwrap_or_default();
        let (first, second, third) = series_colors(&p);
        rows.sort_by(|a, b| b.power_w.total_cmp(&a.power_w));
        if self.energy_rows == 0 {
            self.energy_rows = 10;
        }
        rows.truncate(self.energy_rows);
        self.request_icons(ui, rows.iter());

        // Kopf: aktuelle Leistung, Gesamtenergie mit Emissionen, geschätzte Kosten.
        let currency = shared.settings.power_currency.clone();
        let total_kwh = power.total_wh / 1000.0;
        let emissions = total_kwh * shared.settings.power_co2_g_kwh as f64;
        let cost = total_kwh * shared.settings.power_price_kwh as f64;
        theme::card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.columns(3, |columns| {
                columns[0].vertical_centered(|ui| {
                    ui.label(theme::secondary(&p, t("Aktuelle Leistungsaufnahme")));
                    ui.label(
                        RichText::new(format!("{:.1} W", power.total_w).replace('.', ","))
                            .size(32.0)
                            .color(p.accent),
                    );
                    let quality = match (power.cpu_measured, power.gpu_measured) {
                        (true, true) => "Prozessor und Grafikkarte gemessen",
                        (true, false) => "Prozessor gemessen, Grafikkarte geschätzt",
                        (false, true) => "Grafikkarte gemessen, Prozessor geschätzt",
                        (false, false) => "geschätzt",
                    };
                    ui.label(theme::secondary(&p, quality).size(12.0));
                    // Aufschlüsselung – so lässt sich jeder Anteil einzeln nachvollziehen.
                    let mut parts = vec![
                        format!("CPU {:.1} W", power.cpu_w),
                        format!("GPU {:.1} W", power.gpu_w),
                        format!("RAM {:.1} W", power.ram_w),
                    ];
                    if power.base_w > 0.0 {
                        parts.push(crate::i18n::td(format!("Grundlast {:.0} W", power.base_w)));
                    }
                    ui.label(theme::secondary(&p, parts.join(" + ").replace('.', ",")).size(12.0));
                });
                columns[1].vertical_centered(|ui| {
                    ui.label(theme::secondary(&p, t("Gesamt")));
                    ui.label(
                        RichText::new(format!("{total_kwh:.2} kWh").replace('.', ","))
                            .size(18.0)
                            .color(third),
                    );
                    ui.label(theme::secondary(&p, t("Emissionen")));
                    ui.label(
                        RichText::new(format!("{emissions:.1} g CO₂").replace('.', ","))
                            .size(18.0)
                            .color(second),
                    );
                });
                columns[2].vertical_centered(|ui| {
                    ui.label(theme::secondary(&p, t("Geschätzte Kosten")));
                    ui.label(
                        RichText::new(format!("{cost:.2} {currency}").replace('.', ","))
                            .size(18.0)
                            .color(p.accent),
                    );
                    ui.label(
                        theme::secondary(
                            &p,
                            crate::i18n::td(format!("{:.1} Wh seit dem Start", power.session_wh))
                                .replace('.', ","),
                        )
                        .size(12.0),
                    );
                });
            });
        });

        self.range_chips(ui, &p);

        theme::card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::card_title(ui, t("Gesamt"));
            let max = (headroom(&samples, |s| s.power_w, 50.0) / 50.0).ceil() * 50.0;
            let lines = [Line {
                label: "Leistung",
                color: first,
                value: &|s| Some(s.power_w),
            }];
            chart(ui, &p, &samples, &lines, max, "W", range, now);
        });

        // Programme nach geschätztem Verbrauch.
        theme::card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::card_title(ui, t("Prozesse"));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    for count in [25usize, 10, 5] {
                        if theme::chip(
                            ui,
                            &p,
                            &crate::i18n::td(format!("Top {count}")),
                            self.energy_rows == count,
                        )
                        .clicked()
                        {
                            self.energy_rows = count;
                        }
                    }
                });
            });
            egui::Grid::new("monitor-energy-processes")
                .num_columns(6)
                .striped(true)
                .spacing(egui::vec2(26.0, 6.0))
                .show(ui, |ui| {
                    for title in ["Anwendung", "Leistung", "CPU", "RAM", "Lesen", "Schreiben"] {
                        ui.label(theme::secondary(&p, title).size(12.5));
                    }
                    ui.end_row();
                    for row in &rows {
                        ui.horizontal(|ui| {
                            theme::app_icon(
                                ui,
                                &p,
                                self.icon_of(row),
                                display_name(&row.name),
                                16.0,
                            );
                            let name = format!("{} ({})", display_name(&row.name), row.pids.len());
                            ui.label(RichText::new(name).color(p.accent));
                        });
                        ui.label(
                            RichText::new(format!("{:.1} W", row.power_w).replace('.', ","))
                                .strong(),
                        );
                        ui.label(format_percent(row.cpu));
                        ui.label(format_bytes(row.memory));
                        ui.label(format_rate(row.disk_read));
                        ui.label(format_rate(row.disk_write));
                        ui.end_row();
                    }
                });
        });

        // Komponenten in zwei Spalten.
        let disk_max = headroom(
            &samples,
            |s| s.disk_read.max(s.disk_write) / BYTES_PER_MB,
            1.0,
        );
        let net_max = headroom(&samples, |s| s.net_down.max(s.net_up) / BYTES_PER_MB, 0.5);
        let watt_max =
            |value: fn(&Sample) -> f32| (headroom(&samples, value, 10.0) / 10.0).ceil() * 10.0;
        let (cpu_max, gpu_max, ram_max) = (
            watt_max(|s| s.cpu_w),
            watt_max(|s| s.gpu_w),
            watt_max(|s| s.ram_w),
        );
        ui.columns(2, |columns| {
            theme::card(&p).show(&mut columns[0], |ui| {
                ui.set_width(ui.available_width());
                theme::card_title(ui, "CPU");
                ui.label(theme::secondary(
                    &p,
                    crate::i18n::td(format!(
                        "Leistung: {:.1} W ({}) · Auslastung: {:.1} %",
                        power.cpu_w,
                        measured_label(power.cpu_measured),
                        last.cpu
                    ))
                    .replace('.', ","),
                ));
                let lines = [Line {
                    label: "Leistung",
                    color: first,
                    value: &|s| Some(s.cpu_w),
                }];
                chart(ui, &p, &samples, &lines, cpu_max, "W", range, now);
            });
            theme::card(&p).show(&mut columns[1], |ui| {
                ui.set_width(ui.available_width());
                theme::card_title(ui, "GPU");
                let source = measured_label(power.gpu_measured);
                let usage = last.gpu.map_or("–".to_owned(), |u| format!("{u:.1} %"));
                ui.label(theme::secondary(
                    &p,
                    crate::i18n::td(format!(
                        "Leistung: {:.1} W ({source}) · Auslastung: {usage}",
                        power.gpu_w
                    ))
                    .replace('.', ","),
                ));
                let lines = [Line {
                    label: "Leistung",
                    color: first,
                    value: &|s| Some(s.gpu_w),
                }];
                chart(ui, &p, &samples, &lines, gpu_max, "W", range, now);
            });
            theme::card(&p).show(&mut columns[0], |ui| {
                ui.set_width(ui.available_width());
                theme::card_title(ui, "RAM");
                ui.label(theme::secondary(
                    &p,
                    crate::i18n::td(format!(
                        "Leistung: {:.1} W (geschätzt) · Belegt: {:.1} %",
                        power.ram_w, last.ram
                    ))
                    .replace('.', ","),
                ));
                let lines = [Line {
                    label: "Leistung",
                    color: third,
                    value: &|s| Some(s.ram_w),
                }];
                chart(ui, &p, &samples, &lines, ram_max, "W", range, now);
            });
            theme::card(&p).show(&mut columns[1], |ui| {
                ui.set_width(ui.available_width());
                theme::card_title(ui, t("Datenträger"));
                ui.label(theme::secondary(
                    &p,
                    crate::i18n::td(format!(
                        "Lesen: {} · Schreiben: {}",
                        format_rate(last.disk_read),
                        format_rate(last.disk_write)
                    )),
                ));
                let lines = [
                    Line {
                        label: "Lesen",
                        color: third,
                        value: &|s| Some(s.disk_read / BYTES_PER_MB),
                    },
                    Line {
                        label: "Schreiben",
                        color: second,
                        value: &|s| Some(s.disk_write / BYTES_PER_MB),
                    },
                ];
                chart(ui, &p, &samples, &lines, disk_max, "MB/s", range, now);
            });
            theme::card(&p).show(&mut columns[0], |ui| {
                ui.set_width(ui.available_width());
                theme::card_title(ui, t("Netzwerk"));
                ui.label(theme::secondary(
                    &p,
                    crate::i18n::td(format!(
                        "Empfangen: {} · Gesendet: {}",
                        format_rate(last.net_down),
                        format_rate(last.net_up)
                    )),
                ));
                let lines = [
                    Line {
                        label: "Empfangen",
                        color: third,
                        value: &|s| Some(s.net_down / BYTES_PER_MB),
                    },
                    Line {
                        label: "Gesendet",
                        color: second,
                        value: &|s| Some(s.net_up / BYTES_PER_MB),
                    },
                ];
                chart(ui, &p, &samples, &lines, net_max, "MB/s", range, now);
            });
        });

        // Annahmen der Schätzung.
        let mut changed = false;
        theme::card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::card_title(ui, t("Annahmen"));
            let source = |measured: bool, how: &str, missing: &str| {
                if measured {
                    crate::i18n::td(format!("gemessen ({how})"))
                } else {
                    crate::i18n::td(format!("geschätzt – {missing}"))
                }
            };
            ui.label(crate::i18n::td(format!(
                "Prozessor: {}",
                source(
                    power.cpu_measured,
                    t("Energiezähler von Windows"),
                    t("dieses System bietet Windows keinen Energiezähler an")
                )
            )));
            ui.label(crate::i18n::td(format!(
                "Grafikkarte: {}",
                source(
                    power.gpu_measured,
                    t("Sensor des Grafiktreibers"),
                    t("der Treiber meldet keine Leistung")
                )
            )));
            ui.label(
                theme::secondary(
                    &p,
                    "Gemessene Werte kommen direkt aus den Sensoren von Prozessor und Grafikkarte. Was kein \
                     Sensor erfasst, wird geschätzt: Arbeitsspeicher, Mainboard, Laufwerke, Lüfter und die \
                     Verluste des Netzteils (Grundlast). Die Maximalleistungen unten gelten nur für geschätzte \
                     Komponenten. Den Verbrauch an der Steckdose kann nur ein Messgerät zeigen.",
                )
                .size(12.5),
            );
            let settings = &mut shared.settings;
            let mut field = |ui: &mut Ui, label: &str, value: &mut f32, max: f32, speed: f64, suffix: &str| {
                ui.horizontal(|ui| {
                    ui.label(label);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let drag = egui::DragValue::new(value).range(0.0..=max).speed(speed).suffix(suffix);
                        // Erst speichern, wenn die Eingabe abgeschlossen ist.
                        let response = ui.add(drag);
                        if response.drag_stopped() || response.lost_focus() {
                            changed = true;
                        }
                    });
                });
            };
            field(ui, t("Maximalleistung Prozessor (TDP)"), &mut settings.power_cpu_max_w, 1000.0, 1.0, " W");
            field(ui, t("Maximalleistung Grafikkarte"), &mut settings.power_gpu_max_w, 1500.0, 1.0, " W");
            field(ui, t("Zusätzliche Grundlast (Mainboard, Lüfter, Netzteil)"), &mut settings.power_extra_base_w, 500.0, 1.0, " W");
            field(ui, t("Strompreis je kWh"), &mut settings.power_price_kwh, 5.0, 0.01, "");
            field(ui, t("CO₂ je kWh"), &mut settings.power_co2_g_kwh, 2000.0, 1.0, " g");
            ui.horizontal(|ui| {
                ui.label(t("Währung"));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let response = ui.add(TextEdit::singleline(&mut settings.power_currency).desired_width(60.0));
                    if response.lost_focus() {
                        changed = true;
                    }
                });
            });
        });
        if changed {
            shared.save_settings();
            shared
                .monitor
                .send(Command::Config(shared.settings.power_config()));
        }
    }
}
