use crate::i18n::t;

mod perf;
mod startup_page;

use std::{
    collections::{HashMap, VecDeque},
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
        hwinfo::{self, SystemStats},
        icons::{self, IconImage},
        monitor::{Command, DiskRow, GpuInfo, NetworkRow, ProcessRow, Sample, State},
        smart::{self, SmartInfo, Status},
        winsys::PhysicalDisk,
    },
    theme::{self, Palette},
};

/// So viele Punkte zeichnet ein Diagramm höchstens – mehr sieht man ohnehin nicht.
const MAX_CHART_POINTS: usize = 600;
/// Wählbare Messabstände in Millisekunden.
const INTERVALS: [(u64, &str); 4] = [
    (2000, "alle 2 s"),
    (1000, "jede Sekunde"),
    (500, "alle 0,5 s"),
    (100, "alle 0,1 s"),
];
/// Höhe der Diagramme; wird je Seite so gewählt, dass alles ohne Scrollen passt.
static CHART_HEIGHT_BITS: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0x432A_0000); // 170.0

fn chart_height() -> f32 {
    f32::from_bits(CHART_HEIGHT_BITS.load(std::sync::atomic::Ordering::Relaxed))
}

fn set_chart_height(height: f32) {
    CHART_HEIGHT_BITS.store(height.to_bits(), std::sync::atomic::Ordering::Relaxed);
}

/// Größte Lücke zwischen zwei Messpunkten, die noch überbrückt wird: ein Mehrfaches des
/// üblichen Abstands (Minutenwerte brauchen mehr als Sekundenwerte).
fn gap_limit(samples: &[Sample]) -> f64 {
    let mut gaps: Vec<f64> = samples
        .windows(2)
        .map(|pair| pair[1].secs() - pair[0].secs())
        .filter(|gap| *gap > 0.0)
        .collect();
    if gaps.is_empty() {
        return 90.0;
    }
    gaps.sort_by(f64::total_cmp);
    (gaps[gaps.len() / 2] * 4.0).max(90.0)
}
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
    Startup,
    Disks,
}

impl Page {
    const ALL: [(Page, &'static str); 5] = [
        (Page::Processes, "Prozesse"),
        (Page::Performance, "Leistung"),
        (Page::History, "Verlauf"),
        (Page::Startup, "Autostart"),
        (Page::Disks, "Datenträger"),
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
    /// Frei gewählter Zeitraum (Länge in Sekunden).
    Custom(u64),
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
            Range::Custom(_) => "Eigener Zeitraum",
        }
    }

    fn seconds(self) -> u64 {
        match self {
            Range::Minute => 60,
            Range::FiveMinutes => 300,
            Range::Hour => 3600,
            Range::Day => 86_400,
            Range::Week => 7 * 86_400,
            Range::Custom(seconds) => seconds.max(60),
        }
    }

    /// Zeitraum der Zeitleiste unter dem Verlauf – jeweils deutlich größer als das Fenster.
    fn overview_seconds(self) -> u64 {
        match self {
            Range::Minute => 3600,
            Range::FiveMinutes | Range::Hour => 86_400,
            Range::Day | Range::Week | Range::Custom(_) => 7 * 86_400,
        }
    }

    /// Lange Zeiträume kommen aus dem gespeicherten Minutenverlauf.
    fn uses_minutes(self) -> bool {
        match self {
            Range::Week => true,
            Range::Custom(seconds) => seconds > 2 * 86_400,
            _ => false,
        }
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
    /// Umgekehrte Reihenfolge (ein zweiter Klick auf die Spalte).
    sort_reversed: bool,
    device: Device,
    /// Gewählte Komponente auf der Seite „Verlauf“.
    history_device: Device,
    /// Verlauf: so viele Sekunden liegt das Ende des gezeigten Fensters zurück (0 = live).
    history_offset: u64,
    /// Zeitleiste: gezeigter Zeitraum in Sekunden (0 = automatisch).
    timeline_secs: u64,
    /// Höhe der Zeitleiste in Pixeln (0 = Standard).
    timeline_height: f32,
    /// Frei gewählter Zeitraum (Anfang, Ende) als Zeitstempel.
    custom: Option<(u64, u64)>,
    custom_from: String,
    custom_to: String,
    custom_error: Option<String>,

    search: String,
    /// Name des markierten Programms in der Prozessliste.
    selected: Option<String>,
    /// Wartet auf Bestätigung: Programm (Name, Prozess-IDs) beenden.
    confirm_kill: Option<(String, Vec<u32>)>,
    /// Icons je Programmdatei; `None` = angefragt oder nicht vorhanden.
    icons: HashMap<String, Option<TextureHandle>>,
    icon_loader: Option<IconLoader>,
    /// Offenes Fenster mit den Details eines Datenträgers (Nummer, SMART-Werte).
    disk_popup: Option<(u32, SmartInfo)>,
    /// Seite „Autostart“.
    startup: startup_page::StartupView,
    /// Liest Hardware-Angaben (Cache, Treiber, Adresse …); startet erst bei Bedarf.
    hw: Option<hwinfo::Handle>,
    /// Schalter „Erweitert“ auf der Seite „Leistung“.
    advanced: bool,
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
    // Feinste Stufe, die den Anfang des Zeitraums noch abdeckt: Live-Werte, 30-Sekunden-Werte,
    // zuletzt Minutenwerte.
    let covers =
        |tier: &VecDeque<Sample>| tier.front().is_some_and(|first| start + 60 >= first.time);
    let (source, is_live) = if !coarse && covers(&state.recent) {
        (&state.recent, true)
    } else if !coarse && covers(&state.thirty) {
        (&state.thirty, false)
    } else {
        (&state.minutes, false)
    };
    // Reicht die gröbere Stufe zurück, kommen die feinen Live-Werte des jüngsten Teils dazu.
    let live_from = if coarse || is_live {
        u64::MAX
    } else {
        state.recent.front().map_or(u64::MAX, |first| first.time)
    };
    let mut in_range: Vec<&Sample> = source
        .iter()
        .filter(|sample| sample.time >= start && sample.time <= end && sample.time < live_from)
        .collect();
    if live_from != u64::MAX {
        in_range.extend(
            state
                .recent
                .iter()
                .filter(|sample| sample.time >= start && sample.time <= end),
        );
    }
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

    let size = egui::vec2(ui.available_width(), chart_height());
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, p.radius, p.field_bg);

    let plot = rect.shrink2(egui::vec2(8.0, 10.0));
    let span = range.seconds() as f32;
    let start = now.saturating_sub(range.seconds());
    let x_of = |time: f64| {
        plot.left() + (((time - start as f64) / span as f64) as f32).clamp(0.0, 1.0) * plot.width()
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
    let max_gap = gap_limit(samples).max(range.seconds() as f64 / 40.0);
    for line in lines {
        let mut segment: Vec<Pos2> = Vec::new();
        let mut previous_time = 0.0f64;
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
                    if !segment.is_empty() && sample.secs() - previous_time > max_gap {
                        flush(&mut segment);
                    }
                    segment.push(Pos2::new(x_of(sample.secs()), y_of(value)));
                    previous_time = sample.secs();
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
        (x_of(a.secs()) - pointer.x)
            .abs()
            .total_cmp(&(x_of(b.secs()) - pointer.x).abs())
    });
    let Some(sample) = nearest else {
        return;
    };
    painter.vline(
        x_of(sample.secs()),
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

/// Höchster Wert einer Reihe, mit etwas Luft und Untergrenze – als Diagramm-Obergrenze.
fn headroom(samples: &[Sample], value: impl Fn(&Sample) -> f32, minimum: f32) -> f32 {
    let peak = samples.iter().map(value).fold(0.0f32, f32::max);
    (peak * 1.2).max(minimum)
}

/// Gut sichtbarer Pfeil neben der Spalte, nach der sortiert wird.
/// Gespeicherte Spaltenbreiten, sonst die Standardwerte.
fn column_widths<const N: usize>(saved: &[f32], default: [f32; N]) -> [f32; N] {
    if saved.len() == N {
        let mut widths = default;
        for (width, value) in widths.iter_mut().zip(saved) {
            *width = value.clamp(60.0, 700.0);
        }
        widths
    } else {
        default
    }
}

/// Anfasser zum Verschieben einer Spaltengrenze. Liefert (Verschiebung nach rechts, losgelassen).
fn resize_handle(
    ui: &mut Ui,
    id: egui::Id,
    x: f32,
    top: f32,
    bottom: f32,
    p: &Palette,
) -> (f32, bool) {
    let rect = Rect::from_min_max(Pos2::new(x - 4.0, top), Pos2::new(x + 4.0, bottom));
    let response = ui.interact(rect, id, Sense::drag());
    let active = response.hovered() || response.dragged();
    let color = if active {
        p.text_secondary
    } else {
        p.text_secondary.gamma_multiply(0.3)
    };
    ui.painter()
        .vline(x, (top + 6.0)..=(bottom - 6.0), Stroke::new(1.0_f32, color));
    if active {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
    }
    (response.drag_delta().x, response.drag_stopped())
}

fn sort_arrow(ui: &Ui, center: Pos2, color: Color32, up: bool) {
    let (half_width, half_height) = (6.0, if up { -4.0 } else { 4.0 });
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
    stats: SystemStats,
    disk_latency: Vec<f32>,
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
            stats: state.system,
            disk_latency: state.disk_latency.clone(),
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

/// Anzahl der Diagramme, die ein Gerät zeigt.
fn chart_count(device: Device, samples: &[Sample]) -> usize {
    match device {
        Device::Cpu => 1 + usize::from(samples.iter().any(|s| s.cpu_temp.is_some())),
        Device::Gpu(index) => {
            1 + usize::from(samples.iter().any(|s| gpu_value(s, index, 1).is_some()))
        }
        Device::Disk(_) => 2,
        Device::Memory | Device::Network => 1,
    }
}

/// Wählt die Diagrammhöhe so, dass `charts` Diagramme plus `reserved` Pixel für alles andere
/// in den sichtbaren Rest des Fensters passen – ohne Scrollen.
fn fit_charts(ui: &Ui, charts: usize, reserved: f32) {
    // Je Karte: Titel, Legende, Ränder.
    const CARD_OVERHEAD: f32 = 88.0;
    // Die Höhe wird gemerkt: würde sie in jedem Bild aus der Scrollposition abgeleitet,
    // wüchsen die Diagramme beim Scrollen.
    let key = egui::Id::new((
        "fit-charts",
        ui.clip_rect().height().round() as i32,
        charts,
        reserved.round() as i32,
    ));
    let per_chart = ui
        .data(|data| data.get_temp::<f32>(key))
        .unwrap_or_else(|| {
            let remaining = (ui.clip_rect().bottom() - ui.next_widget_position().y).max(0.0);
            let height =
                ((remaining - reserved) / charts.max(1) as f32 - CARD_OVERHEAD).clamp(110.0, 420.0);
            ui.data_mut(|data| data.insert_temp(key, height));
            height
        });
    set_chart_height(per_chart);
}

/// Wählbare Aufbewahrungsdauer des Verlaufs in Tagen.
const RETENTION_DAYS: [u32; 5] = [1, 3, 7, 14, 30];

fn retention_label(days: u32) -> String {
    if days == 1 {
        t("1 Tag").to_owned()
    } else {
        crate::i18n::td(format!("{days} Tage"))
    }
}

/// Wählbarer Zeitraum der Zeitleiste (Sekunden, Beschriftung).
const TIMELINE_SPANS: [(u64, &str); 8] = [
    (0, "Automatisch"),
    (3600, "1 Stunde"),
    (21_600, "6 Stunden"),
    (86_400, "24 Stunden"),
    (259_200, "3 Tage"),
    (604_800, "7 Tage"),
    (1_209_600, "14 Tage"),
    (2_592_000, "30 Tage"),
];

/// Tage seit 1970 für ein Kalenderdatum (Algorithmus nach Howard Hinnant).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Liest einen Zeitpunkt in Ortszeit: `10.12.2025 14:30`, `10.12.2025`, `10.12. 14:30`,
/// `2025-12-10 14:30` oder nur `10:00` (heute). Ohne Uhrzeit gilt der Tagesanfang, am Ende
/// eines Zeitraums (`end_of_range`) das Tagesende. `today` ist (Jahr, Monat, Tag).
fn parse_moment(
    text: &str,
    today: (i64, i64, i64),
    offset: i64,
    end_of_range: bool,
) -> Option<u64> {
    if text.trim().is_empty() {
        return None;
    }
    let mut date: Option<(i64, i64, i64)> = None;
    let mut time: Option<(i64, i64, i64)> = None;
    for part in text.split_whitespace() {
        if part.contains(':') {
            let mut fields = part.split(':');
            let hour: i64 = fields.next()?.parse().ok()?;
            let minute: i64 = fields.next()?.parse().ok()?;
            let second: i64 = match fields.next() {
                Some(field) => field.parse().ok()?,
                None => 0,
            };
            time = Some((hour, minute, second));
        } else if part.contains('.') || part.contains('-') {
            let numbers: Vec<i64> = part
                .split(['.', '-'])
                .filter(|field| !field.is_empty())
                .map(|field| field.parse().ok())
                .collect::<Option<Vec<_>>>()?;
            date = Some(match numbers.as_slice() {
                [first, month, day] if *first > 31 => (*first, *month, *day),
                [day, month, year] => {
                    (if *year < 100 { 2000 + *year } else { *year }, *month, *day)
                }
                [day, month] => (today.0, *month, *day),
                _ => return None,
            });
        } else {
            return None;
        }
    }
    let (year, month, day) = date.unwrap_or(today);
    let (hour, minute, second) = time.unwrap_or((0, 0, 0));
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(0..24).contains(&hour)
        || !(0..60).contains(&minute)
        || !(0..60).contains(&second)
        || !(1970..=2200).contains(&year)
    {
        return None;
    }
    let mut seconds =
        days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset;
    if time.is_none() && end_of_range {
        seconds += 86_400;
    }
    u64::try_from(seconds).ok()
}

/// `10.12.2025 14:30` in Ortszeit.
fn format_full(time: u64) -> String {
    let (year, month, day, hour, minute) = platform::local_parts(time);
    format!("{day:02}.{month:02}.{year} {hour:02}:{minute:02}")
}

/// Die Diagramme eines Geräts.
fn device_charts(
    ui: &mut Ui,
    p: &Palette,
    device: Device,
    samples: &[Sample],
    range: Range,
    now: u64,
) {
    let (first, second, third) = series_colors(p);
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

impl MonitorView {
    /// Ansicht aktualisieren: Autostart, Laufwerke und Netzwerkangaben neu lesen.
    pub fn refresh(&mut self, ctx: &egui::Context, shared: &mut Shared) {
        self.startup.reload(ctx);
        if let Some(hw) = &self.hw {
            hw.refresh_network();
        }
        shared.monitor.send(Command::RefreshDisks);
    }

    /// Seitenwahl; steht fest oben, außerhalb des Scrollbereichs.
    pub fn header(&mut self, ui: &mut Ui, shared: &mut Shared) {
        let p = shared.palette;
        ui.horizontal(|ui| {
            for (page, label) in Page::ALL {
                if theme::chip(ui, &p, t(label), self.page == page).clicked() {
                    self.page = page;
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let current = INTERVALS
                    .iter()
                    .find(|(millis, _)| *millis == shared.settings.monitor_interval_ms)
                    .map_or("Eigene", |(_, label)| *label);
                let mut changed = false;
                egui::ComboBox::from_id_salt("monitor-interval")
                    .selected_text(t(current))
                    .show_ui(ui, |ui| {
                        for (millis, label) in INTERVALS {
                            changed |= ui
                                .selectable_value(
                                    &mut shared.settings.monitor_interval_ms,
                                    millis,
                                    t(label),
                                )
                                .clicked();
                        }
                    });
                ui.label(theme::secondary(&p, t("Aktualisierung:")).size(12.5));
                if changed {
                    shared.save_settings();
                }
            });
        });
        ui.add_space(4.0);
    }

    pub fn ui(&mut self, ui: &mut Ui, shared: &mut Shared) {
        if let Some(message) = shared.monitor.take_message() {
            shared.notify(message);
        }
        self.receive_icons(ui);

        match self.page {
            Page::Processes => self.processes(ui, shared),
            Page::Performance => self.performance(ui, shared),
            Page::History => self.history(ui, shared),
            Page::Startup => self.startup.ui(ui, shared),
            Page::Disks => self.disks(ui, shared),
        }
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
        let needle = self.search.trim().to_lowercase();
        if !needle.is_empty() {
            rows.retain(|row| row.name.to_lowercase().contains(&needle));
        }
        let total = rows.len();
        match self.sort {
            Sort::Cpu => rows.sort_by(|a, b| b.cpu.total_cmp(&a.cpu)),
            Sort::Memory => rows.sort_by(|a, b| b.memory.cmp(&a.memory)),
            Sort::Disk => rows.sort_by(|a, b| {
                (b.disk_read + b.disk_write).total_cmp(&(a.disk_read + a.disk_write))
            }),
            Sort::Name => rows.sort_by_key(|row| row.name.to_lowercase()),
        }
        if self.sort_reversed {
            rows.reverse();
        }
        rows.truncate(MAX_PROCESS_ROWS);
        self.request_icons(ui, rows.iter());

        // Die Markierung verfällt, wenn das Programm nicht mehr läuft.
        let selected_row = self
            .selected
            .as_ref()
            .and_then(|name| rows.iter().find(|row| &row.name == name))
            .cloned();
        if selected_row.is_none()
            || (self.confirm_kill.is_none()
                && ui.input(|input| input.key_pressed(egui::Key::Escape)))
        {
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
                if ui
                    .add_enabled(
                        selected_row.is_some(),
                        egui::Button::new(t("Auswahl aufheben")),
                    )
                    .on_hover_text(t("Hebt die Markierung in der Liste auf (Esc)"))
                    .clicked()
                {
                    self.selected = None;
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
        let mut widths = column_widths(&shared.settings.process_columns, [110.0, 120.0, 110.0]);
        let mut widths_changed = false;
        let mut widths_saved = false;
        theme::card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 1.0;

            // Spaltenköpfe wie im Task-Manager: oben die Gesamtauslastung, darunter der Name.
            let (header, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 40.0), Sense::hover());
            let disk_right = header.right() - 12.0;
            let memory_right = disk_right - widths[2];
            let cpu_right = memory_right - widths[1];
            let name_right = cpu_right - widths[0];
            let columns = [
                ("CPU", format!("{cpu_total:.0} %"), Sort::Cpu, cpu_right, 0),
                (
                    "Arbeitsspeicher",
                    format!("{ram_total:.0} %"),
                    Sort::Memory,
                    memory_right,
                    1,
                ),
                (
                    "Datenträger",
                    format_rate(disk_total),
                    Sort::Disk,
                    disk_right,
                    2,
                ),
            ];
            for (label, value, sort, right, index) in columns {
                let rect = Rect::from_min_max(
                    Pos2::new(right - widths[index] + 8.0, header.top()),
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
                        self.sort_reversed,
                    );
                }
                if response.clicked() {
                    if self.sort == sort {
                        self.sort_reversed = !self.sort_reversed;
                    } else {
                        self.sort = sort;
                        self.sort_reversed = false;
                    }
                }
                let room = (header.width() - 24.0 - 150.0 - widths.iter().sum::<f32>()
                    + widths[index])
                    .max(60.0);
                let (delta, stopped) = resize_handle(
                    ui,
                    ui.id().with(("process-resize", label)),
                    right - widths[index],
                    header.top(),
                    header.bottom(),
                    &p,
                );
                if delta != 0.0 {
                    widths[index] = (widths[index] - delta).clamp(60.0, room.min(700.0));
                    widths_changed = true;
                }
                widths_saved |= stopped;
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
                    self.sort_reversed,
                );
            }
            if name_response.clicked() {
                if self.sort == Sort::Name {
                    self.sort_reversed = !self.sort_reversed;
                } else {
                    self.sort = Sort::Name;
                    self.sort_reversed = false;
                }
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
                            (cpu_right, 0, format_percent(row.cpu), row.cpu / 50.0),
                            (
                                memory_right,
                                1,
                                format_bytes(row.memory),
                                row.memory as f32 / 4.0e9,
                            ),
                            (
                                disk_right,
                                2,
                                format_rate(disk),
                                disk / (50.0 * BYTES_PER_MB),
                            ),
                        ];
                        for (right, index, text, heat) in cells {
                            let cell = Rect::from_min_max(
                                Pos2::new(right - widths[index] + 10.0, rect.top() + 1.0),
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
        if widths_changed {
            shared.settings.process_columns = widths.to_vec();
        }
        if widths_saved {
            shared.save_settings();
        }
        if let Some(err) = open_error {
            shared.notify(crate::i18n::td(format!("Dateisystem: {err}")));
        }
    }

    // ───────── Leistung (wie im Task-Manager) ─────────

    fn performance(&mut self, ui: &mut Ui, shared: &mut Shared) {
        let p = shared.palette;
        let now = unix_now();
        let want_cores = self.advanced && self.device == Device::Cpu;
        let (samples, hardware, cores) = shared.monitor.read(|state| {
            (
                collect_samples(state, Range::Minute, now),
                Hardware::from_state(state),
                if want_cores {
                    state.core_history.iter().cloned().collect::<Vec<_>>()
                } else {
                    Vec::new()
                },
            )
        });
        let last = samples.last().cloned().unwrap_or_default();
        let devices = hardware.devices(&last);
        if !devices.contains(&self.device) {
            self.device = Device::Cpu;
        }
        let on_network = self.device == Device::Network;
        let hw_info = {
            let ctx = ui.ctx().clone();
            let handle = self.hw.get_or_insert_with(|| hwinfo::Handle::spawn(ctx));
            if on_network {
                handle.refresh_network();
            }
            handle.get()
        };
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
                        if theme::chip(ui, &p, t("Erweitert"), self.advanced)
                            .on_hover_text(t(
                                "Zeigt einzelne Kerne, Cache-Größen und weitere Angaben",
                            ))
                            .clicked()
                        {
                            self.advanced = !self.advanced;
                        }
                        ui.add(
                            egui::Label::new(theme::secondary(&p, hardware.model(device)))
                                .truncate(),
                        );
                    });
                });
                let info = perf::Info {
                    p: &p,
                    device,
                    hardware: &hardware,
                    hw: &hw_info,
                    samples: &samples,
                    last: &last,
                    advanced: self.advanced,
                    cores: &cores,
                };
                perf::body(ui, &info);
            });
        });
    }

    // ───────── Verlauf je Komponente ─────────

    fn history(&mut self, ui: &mut Ui, shared: &mut Shared) {
        let p = shared.palette;
        let now = unix_now();
        let retention = u64::from(shared.settings.history_days.clamp(1, 30)) * 86_400;
        if self.timeline_height < 40.0 {
            self.timeline_height = 90.0;
        }
        let timeline_height = self.timeline_height.clamp(60.0, 320.0);

        // Gezeigtes Fenster: frei gewählter Zeitraum oder die Voreinstellung bis „jetzt“.
        let range = match self.custom {
            Some((from, to)) => Range::Custom(to.saturating_sub(from)),
            None => self.range,
        };
        let window = range.seconds();
        let mut span = if self.timeline_secs == 0 {
            range.overview_seconds()
        } else {
            self.timeline_secs
        };
        if let Some((from, _)) = self.custom {
            span = span.max(now.saturating_sub(from) + window / 20 + 60);
        }
        span = span.min(retention).max(600);
        let (start, end) = match self.custom {
            Some((from, to)) => (from, to),
            None => {
                // Das Fenster darf höchstens bis zum Anfang der Zeitleiste zurückreichen.
                self.history_offset = self.history_offset.min(span.saturating_sub(window));
                let end = now.saturating_sub(self.history_offset);
                (end.saturating_sub(window), end)
            }
        };
        let (samples, overview, hardware) = shared.monitor.read(|state| {
            (
                collect_between(
                    state,
                    start,
                    end,
                    range.uses_minutes() || start < now.saturating_sub(172_800),
                ),
                collect_between(state, now.saturating_sub(span), now, span > 172_800),
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
                .selected_text(t(range.label()))
                .show_ui(ui, |ui| {
                    for option in Range::ALL {
                        if ui
                            .selectable_value(&mut self.range, option, t(option.label()))
                            .clicked()
                        {
                            self.custom = None;
                            self.custom_error = None;
                        }
                    }
                });
            ui.add_space(6.0);
            ui.label(theme::secondary(&p, t("Gespeichert:")).size(12.5));
            let days = shared.settings.history_days;
            let mut changed = false;
            egui::ComboBox::from_id_salt("monitor-history-retention")
                .selected_text(retention_label(days))
                .show_ui(ui, |ui| {
                    for option in RETENTION_DAYS {
                        changed |= ui
                            .selectable_value(
                                &mut shared.settings.history_days,
                                option,
                                retention_label(option),
                            )
                            .clicked();
                    }
                });
            if changed {
                shared.save_settings();
            }
        });

        // Zeitleiste und frei wählbarer Zeitraum.
        ui.horizontal_wrapped(|ui| {
            ui.label(theme::secondary(&p, t("Zeitleiste:")).size(12.5));
            let selected = TIMELINE_SPANS
                .iter()
                .find(|(secs, _)| *secs == self.timeline_secs)
                .map_or("Automatisch", |(_, label)| *label);
            egui::ComboBox::from_id_salt("monitor-timeline-span")
                .selected_text(t(selected))
                .show_ui(ui, |ui| {
                    for (secs, label) in TIMELINE_SPANS {
                        ui.selectable_value(&mut self.timeline_secs, secs, t(label));
                    }
                });
            ui.label(theme::secondary(&p, t("Höhe:")).size(12.5));
            ui.spacing_mut().slider_width = 90.0;
            ui.add(egui::Slider::new(&mut self.timeline_height, 60.0..=320.0).show_value(false));
            ui.add_space(10.0);
            ui.label(theme::secondary(&p, t("Von")).size(12.5));
            ui.add(
                TextEdit::singleline(&mut self.custom_from)
                    .hint_text("TT.MM.JJJJ HH:MM")
                    .desired_width(130.0),
            );
            ui.label(theme::secondary(&p, t("bis")).size(12.5));
            ui.add(
                TextEdit::singleline(&mut self.custom_to)
                    .hint_text("TT.MM.JJJJ HH:MM")
                    .desired_width(130.0),
            );
            let today = platform::local_parts(now);
            let offset = platform::local_offset_secs();
            let mut apply = false;
            if ui
                .button(t("Anwenden"))
                .on_hover_text(t(
                    "Beispiele: 10.12.2025 – 11.12.2025 (ganze Tage) oder 10:00 – 11:00 (heute)",
                ))
                .clicked()
            {
                apply = true;
            }
            let midnight = days_from_civil(today.0, today.1, today.2) * 86_400 - offset;
            let mut quick: Option<(u64, u64)> = None;
            if ui.button(t("Heute")).clicked() {
                quick = Some((midnight.max(0) as u64, now));
            }
            if ui.button(t("Gestern")).clicked() {
                quick = Some(((midnight - 86_400).max(0) as u64, midnight.max(0) as u64));
            }
            if ui
                .add_enabled(self.custom.is_some(), egui::Button::new(t("Zurücksetzen")))
                .clicked()
            {
                self.custom = None;
                self.custom_error = None;
                self.custom_from.clear();
                self.custom_to.clear();
            }
            if let Some((from, to)) = quick {
                self.custom_from = format_full(from);
                self.custom_to = format_full(to);
                apply = true;
            }
            if apply {
                let from = parse_moment(
                    &self.custom_from,
                    (today.0, today.1, today.2),
                    offset,
                    false,
                );
                let to = if self.custom_to.trim().is_empty() {
                    Some(now)
                } else {
                    parse_moment(&self.custom_to, (today.0, today.1, today.2), offset, true)
                };
                self.custom_error = None;
                match (from, to) {
                    (Some(from), Some(to)) if to <= from => {
                        self.custom_error =
                            Some("Das Ende muss nach dem Anfang liegen.".to_owned());
                    }
                    (Some(from), _) if from >= now => {
                        self.custom_error = Some("Der Anfang liegt in der Zukunft.".to_owned());
                    }
                    (Some(from), Some(to)) => {
                        self.custom = Some((from, to.min(now)));
                        self.history_offset = 0;
                    }
                    _ => {
                        self.custom_error = Some(
                            "Datum nicht verstanden. Beispiele: 10.12.2025, 10.12.2025 14:30, 10:00"
                                .to_owned(),
                        );
                    }
                }
            }
        });
        if let Some(error) = &self.custom_error {
            ui.label(RichText::new(t(error)).size(12.5).color(p.danger));
        }

        fit_charts(
            ui,
            chart_count(self.history_device, &samples),
            timeline_height + 150.0,
        );
        device_charts(ui, &p, self.history_device, &samples, range, end);

        // ── Zeitleiste: größerer Zeitraum, das gezeigte Fenster lässt sich verschieben ──
        theme::card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let step = (window / 2).max(1);
                if ui.button("◀").on_hover_text(t("Früher")).clicked() {
                    match self.custom {
                        Some((from, to)) => {
                            self.custom =
                                Some((from.saturating_sub(step), to.saturating_sub(step)));
                        }
                        None => self.history_offset += step,
                    }
                }
                let can_forward =
                    self.history_offset > 0 || self.custom.is_some_and(|(_, to)| to < now);
                if ui
                    .add_enabled(can_forward, egui::Button::new("▶"))
                    .on_hover_text(t("Später"))
                    .clicked()
                {
                    match self.custom {
                        Some((from, to)) => {
                            let to = (to + step).min(now);
                            self.custom = Some((to.saturating_sub(window), to));
                            let _ = from;
                        }
                        None => self.history_offset = self.history_offset.saturating_sub(step),
                    }
                }
                if ui
                    .add_enabled(can_forward, egui::Button::new(t("Jetzt")))
                    .clicked()
                {
                    self.history_offset = 0;
                    if self.custom.is_some() {
                        self.custom = Some((now.saturating_sub(window), now));
                    }
                }
                let with_date = span > 86_400 || window > 86_400;
                let text = if self.history_offset == 0 && self.custom.is_none() {
                    crate::i18n::td(format!("Live · letzte {}", range.label()))
                } else {
                    format!(
                        "{} – {}",
                        format_clock(start, with_date),
                        format_clock(end, with_date)
                    )
                };
                ui.label(theme::secondary(&p, text).size(12.5));
            });

            let size = egui::vec2(ui.available_width(), timeline_height);
            let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, p.radius, p.field_bg);
            let mut plot = rect.shrink2(egui::vec2(6.0, 4.0));
            plot.max.y -= 14.0;
            let first = now.saturating_sub(span);
            let x_of = |time: f64| {
                plot.left()
                    + (((time - first as f64) / span as f64) as f32).clamp(0.0, 1.0) * plot.width()
            };

            // Zeitmarken mit Datum und Uhrzeit.
            let offset = platform::local_offset_secs();
            let steps = [
                60u64, 300, 600, 900, 1800, 3600, 7200, 10_800, 21_600, 43_200, 86_400, 172_800,
                604_800,
            ];
            let tick_step = steps
                .iter()
                .copied()
                .find(|step| plot.width() * *step as f32 / span as f32 >= 120.0)
                .unwrap_or(604_800) as i64;
            let tick_font = FontId::proportional(10.5);
            let mut tick = ((first as i64 + offset).div_euclid(tick_step) + 1) * tick_step - offset;
            let grid = Stroke::new(1.0_f32, p.text_secondary.gamma_multiply(0.18));
            while tick <= now as i64 {
                let x = x_of(tick as f64);
                painter.vline(x, plot.y_range(), grid);
                painter.text(
                    Pos2::new(x, rect.bottom() - 1.0),
                    Align2::CENTER_BOTTOM,
                    format_clock(tick as u64, span > 86_400 || tick_step >= 86_400),
                    tick_font.clone(),
                    p.text_secondary,
                );
                tick += tick_step;
            }

            // Verlauf der gewählten Komponente über den ganzen Zeitraum.
            let net_max = headroom(&overview, |s| (s.net_down + s.net_up) / BYTES_PER_MB, 0.5);
            let value = device_load(self.history_device, net_max);
            let color = device_color(&p, self.history_device);
            let max_gap = gap_limit(&overview).max(span as f64 / 60.0);
            let mut segment: Vec<Pos2> = Vec::new();
            let mut previous = 0.0f64;
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
                if !segment.is_empty() && sample.secs() - previous > max_gap {
                    flush(&mut segment);
                }
                let y = plot.bottom() - (load / 100.0).clamp(0.0, 1.0) * plot.height();
                segment.push(Pos2::new(x_of(sample.secs()), y));
                previous = sample.secs();
            }
            flush(&mut segment);

            // Gezeigtes Fenster hervorheben.
            let selection = Rect::from_min_max(
                Pos2::new(x_of(start as f64), rect.top() + 1.0),
                Pos2::new(
                    x_of(end as f64).max(x_of(start as f64) + 4.0),
                    rect.bottom() - 1.0,
                ),
            );
            painter.rect_filled(selection, 3.0, p.accent.gamma_multiply(0.14));
            painter.rect_stroke(selection, 3.0, Stroke::new(1.5_f32, p.accent));

            // Klicken oder Ziehen setzt die Mitte des Fensters auf den Mauszeiger.
            let pointer = response.interact_pointer_pos();
            if let Some(pointer) = pointer.filter(|_| response.clicked() || response.dragged()) {
                let share = ((pointer.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
                let center = first + (share as f64 * span as f64) as u64;
                if self.custom.is_some() {
                    let to = (center + window / 2).min(now);
                    self.custom = Some((to.saturating_sub(window), to));
                } else {
                    let wanted_end = (center + window / 2).min(now);
                    let offset = now - wanted_end;
                    // Nahe am rechten Rand rastet das Fenster wieder auf „live“ ein.
                    self.history_offset = if offset < window / 20 { 0 } else { offset };
                }
            }
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
            }
            if let Some(hover) = response.hover_pos() {
                let share = ((hover.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
                let time = first + (share as f64 * span as f64) as u64;
                response.on_hover_text(format_full(time));
            }
        });

        ui.label(
            theme::secondary(
                &p,
                "Zeige mit der Maus auf einen Zeitpunkt, um zu sehen, welches Programm dort die meiste Last erzeugte. \
                 In der Zeitleiste verschiebst du das Fenster per Klick oder Ziehen. Mit „Von“ und „bis“ wählst du \
                 einen beliebigen Zeitraum, zum Beispiel 10.12.2025 bis 11.12.2025 oder heute 10:00 bis 11:00. \
                 Aufgezeichnet wird auch, während Toolless im Tray läuft.",
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
        ui.add_space(4.0);

        if physical.is_empty() {
            ui.label(theme::secondary(
                &p,
                t(if cfg!(windows) {
                    "Der Aufbau der physischen Datenträger wird gelesen …"
                } else {
                    "Datenträger werden gelesen …"
                }),
            ));
        }

        // Datenträger als Kästen, nebeneinander und nach unten fortlaufend – je nach Breite.
        let columns = ((ui.available_width() / 340.0).floor() as usize).clamp(1, 4);
        let mut open: Option<u32> = None;
        ui.columns(columns, |cols| {
            for (index, disk) in physical.iter().enumerate() {
                if disk_card(&mut cols[index % columns], &p, disk, &volumes) {
                    open = Some(disk.number);
                }
            }
        });
        if let Some(number) = open {
            self.disk_popup = Some((number, smart::read(number)));
        }
        self.disk_window(ui.ctx(), &p, &physical, &volumes);

        ui.add_space(6.0);
        ui.label(
            theme::secondary(
                &p,
                t("Klick auf einen Datenträger zeigt Details und Gesundheitswerte (SMART). Partitionen ändern, formatieren oder Laufwerksbuchstaben vergeben geht bewusst nur in der Windows-Datenträgerverwaltung."),
            )
            .size(12.0),
        );

        if let Some(err) = error {
            shared.notify(crate::i18n::td(format!("Dateisystem: {err}")));
        }
    }

    /// Fenster mit den Details eines Datenträgers, wie CrystalDiskInfo.
    fn disk_window(
        &mut self,
        ctx: &egui::Context,
        p: &Palette,
        physical: &[PhysicalDisk],
        volumes: &[DiskRow],
    ) {
        let Some((number, info)) = self.disk_popup.clone() else {
            return;
        };
        let disk = physical.iter().find(|disk| disk.number == number);
        let (mut used, mut free) = (0u64, 0u64);
        let mut letters: Vec<String> = Vec::new();
        for partition in disk.map(|disk| disk.partitions.as_slice()).unwrap_or(&[]) {
            if let Some(volume) = volume_of(volumes, &partition.letter) {
                used += volume.total.saturating_sub(volume.free);
                free += volume.free;
                letters.push(partition.letter.clone());
            }
        }
        let model = if info.model.is_empty() {
            disk.map(|disk| disk.model.clone()).unwrap_or_default()
        } else {
            info.model.clone()
        };
        let (mut close, mut refresh) = (false, false);
        let amber = Color32::from_rgb(230, 160, 0);

        egui::Window::new(crate::i18n::td(format!("Datenträger {number}")))
            .id(egui::Id::new(("disk-window", number)))
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_width(660.0);
                let size = disk.map_or(0, |disk| disk.size);
                ui.label(
                    RichText::new(format!("{model} : {}", format_bytes(size)))
                        .size(20.0)
                        .strong(),
                );
                ui.add_space(6.0);

                ui.horizontal_top(|ui| {
                    // Links: Gesamtzustand und Temperatur als große Felder.
                    ui.vertical(|ui| {
                        let status_color = match info.status {
                            Status::Good => p.accent,
                            Status::Caution => amber,
                            Status::Bad => p.danger,
                            Status::Unknown => p.text_secondary,
                        };
                        let big = |ui: &mut Ui, title: &str, value: String, color: Color32| {
                            let (rect, _) =
                                ui.allocate_exact_size(egui::vec2(130.0, 70.0), Sense::hover());
                            ui.painter()
                                .rect_filled(rect, p.radius, color.gamma_multiply(0.25));
                            ui.painter()
                                .rect_stroke(rect, p.radius, Stroke::new(1.5_f32, color));
                            ui.painter().text(
                                egui::pos2(rect.center().x, rect.top() + 6.0),
                                Align2::CENTER_TOP,
                                title,
                                FontId::proportional(11.5),
                                p.text_secondary,
                            );
                            ui.painter().text(
                                egui::pos2(rect.center().x, rect.bottom() - 8.0),
                                Align2::CENTER_BOTTOM,
                                value,
                                FontId::proportional(22.0),
                                p.text,
                            );
                        };
                        let status = match info.health_percent {
                            Some(percent) => format!("{} {percent} %", t(info.status.label())),
                            None => t(info.status.label()).to_owned(),
                        };
                        big(ui, t("Gesamtzustand"), status, status_color);
                        ui.add_space(6.0);
                        let temperature = info
                            .temperature
                            .map_or("–".to_owned(), |degrees| format!("{degrees:.0} °C"));
                        let temperature_color = match info.temperature {
                            Some(degrees) if degrees >= 65.0 => p.danger,
                            Some(degrees) if degrees >= 50.0 => amber,
                            Some(_) => p.accent,
                            None => p.text_secondary,
                        };
                        big(ui, t("Temperatur"), temperature, temperature_color);
                    });

                    // Rechts: Kennzahlen in vier Spalten.
                    let number_text = |value: Option<u64>, unit: &str| {
                        value.map_or("–".to_owned(), |value| format!("{value}{unit}"))
                    };
                    let bytes_text =
                        |value: Option<u64>| value.map_or("–".to_owned(), format_bytes);
                    let hours = info.power_on_hours.map_or("–".to_owned(), |hours| {
                        format!("{hours} h ({} d)", hours / 24)
                    });
                    let rows: [(&str, String, &str, String); 7] = [
                        (
                            "Firmware",
                            or_dash(&info.firmware),
                            "Gesamt gelesen",
                            bytes_text(info.total_read),
                        ),
                        (
                            "Seriennummer",
                            or_dash(&info.serial),
                            "Gesamt geschrieben",
                            bytes_text(info.total_written),
                        ),
                        (
                            "Schnittstelle",
                            or_dash(if info.interface.is_empty() {
                                disk.map_or("", |disk| disk.bus.as_str())
                            } else {
                                info.interface.as_str()
                            }),
                            "Eingeschaltet",
                            number_text(info.power_cycles, "×"),
                        ),
                        (
                            "Laufwerke",
                            or_dash(&letters.join(" ")),
                            "Betriebsstunden",
                            hours,
                        ),
                        (
                            "Belegt",
                            if used + free > 0 {
                                format_bytes(used)
                            } else {
                                "–".to_owned()
                            },
                            "Unsichere Abschaltungen",
                            number_text(info.unsafe_shutdowns, ""),
                        ),
                        (
                            "Frei",
                            if used + free > 0 {
                                format_bytes(free)
                            } else {
                                "–".to_owned()
                            },
                            "Fehler (Medium)",
                            number_text(info.errors, ""),
                        ),
                        (
                            "Partitionen",
                            disk.map_or("–".to_owned(), |disk| disk.partitions.len().to_string()),
                            "Fehlerprotokoll-Einträge",
                            number_text(info.error_log_entries, ""),
                        ),
                    ];
                    egui::Grid::new("disk-facts")
                        .num_columns(4)
                        .spacing(egui::vec2(14.0, 7.0))
                        .show(ui, |ui| {
                            for (left, left_value, right, right_value) in rows {
                                ui.label(theme::secondary(p, t(left)).size(12.5));
                                ui.label(RichText::new(left_value).strong());
                                ui.label(theme::secondary(p, t(right)).size(12.5));
                                ui.label(RichText::new(right_value).strong());
                                ui.end_row();
                            }
                        });
                });

                ui.add_space(8.0);
                if info.rows.is_empty() {
                    if let Some(note) = &info.note {
                        ui.label(theme::secondary(p, t(note)).size(12.5));
                    }
                } else {
                    egui::ScrollArea::vertical()
                        .id_salt("disk-smart-rows")
                        .max_height(240.0)
                        .show(ui, |ui| {
                            egui::Grid::new("disk-smart")
                                .num_columns(3)
                                .striped(true)
                                .spacing(egui::vec2(20.0, 4.0))
                                .min_col_width(40.0)
                                .show(ui, |ui| {
                                    for title in ["ID", "Attributname", "Rohwerte"] {
                                        ui.label(theme::secondary(p, t(title)).size(12.0));
                                    }
                                    ui.end_row();
                                    for (id, name, raw) in &info.rows {
                                        ui.label(id.as_str());
                                        ui.label(name.as_str());
                                        ui.label(RichText::new(raw.as_str()).monospace());
                                        ui.end_row();
                                    }
                                });
                        });
                }
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(t("Aktualisieren")).clicked() {
                        refresh = true;
                    }
                    if ui.button(t("Schließen")).clicked() {
                        close = true;
                    }
                });
            });

        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            close = true;
        }
        if close {
            self.disk_popup = None;
        } else if refresh {
            self.disk_popup = Some((number, smart::read(number)));
        }
    }
}

fn or_dash(text: &str) -> String {
    if text.is_empty() {
        "–".to_owned()
    } else {
        text.to_owned()
    }
}

/// Das Volume zu einem Laufwerksbuchstaben wie `C:`.
fn volume_of<'a>(volumes: &'a [DiskRow], letter: &str) -> Option<&'a DiskRow> {
    if letter.is_empty() {
        return None;
    }
    volumes
        .iter()
        .find(|volume| volume.mount.starts_with(letter))
}

/// Kasten eines physischen Datenträgers: Name, Größe, Belegung je Laufwerk. Liefert `true`
/// bei Klick.
fn disk_card(ui: &mut Ui, p: &Palette, disk: &PhysicalDisk, volumes: &[DiskRow]) -> bool {
    let inner = theme::card(p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(crate::i18n::td(format!("Datenträger {}", disk.number)))
                    .strong()
                    .size(16.0),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let mut tags = vec![disk.bus.clone()];
                if disk.removable {
                    tags.push(t("Wechselmedium").to_owned());
                }
                tags.retain(|tag| !tag.is_empty());
                ui.label(theme::secondary(p, tags.join(" · ")).size(12.0));
            });
        });
        if !disk.model.is_empty() {
            ui.label(theme::secondary(p, disk.model.as_str()).size(12.5));
        }
        ui.label(
            theme::secondary(p, format!("{} · {}", format_bytes(disk.size), disk.style)).size(12.0),
        );
        ui.add_space(4.0);

        // Gesamtbalken: Partitionen im Verhältnis ihrer Größe, belegter Teil in Akzentfarbe.
        let (rect, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 12.0), Sense::hover());
        let total = disk.size.max(1) as f32;
        let mut x = rect.left();
        for partition in &disk.partitions {
            let width = (rect.width() * partition.size as f32 / total)
                .max(2.0)
                .min(rect.right() - x);
            if width <= 0.0 {
                break;
            }
            let segment =
                Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(width, rect.height()));
            x += width;
            let background = if partition.unallocated {
                p.separator.gamma_multiply(0.5)
            } else {
                p.field_bg
            };
            ui.painter()
                .rect_filled(segment.shrink2(egui::vec2(0.5, 0.0)), 2.0, background);
            if let Some(volume) = volume_of(volumes, &partition.letter) {
                let used = 1.0 - volume.free as f32 / volume.total.max(1) as f32;
                let mut filled = segment.shrink2(egui::vec2(0.5, 0.0));
                filled.set_width(filled.width() * used.clamp(0.0, 1.0));
                let color = if used > 0.9 { p.danger } else { p.accent };
                ui.painter().rect_filled(filled, 2.0, color);
            } else if !partition.unallocated {
                ui.painter().rect_filled(
                    segment.shrink2(egui::vec2(0.5, 0.0)),
                    2.0,
                    p.text_secondary.gamma_multiply(0.5),
                );
            }
        }
        ui.add_space(4.0);

        // Eine Zeile je Laufwerk.
        let mut any = false;
        for partition in &disk.partitions {
            let Some(volume) = volume_of(volumes, &partition.letter) else {
                continue;
            };
            any = true;
            let used = volume.total.saturating_sub(volume.free);
            let share = used as f32 / volume.total.max(1) as f32;
            ui.horizontal(|ui| {
                ui.label(RichText::new(partition.letter.as_str()).strong());
                if !volume.name.is_empty() {
                    ui.label(theme::secondary(p, volume.name.as_str()).size(12.5));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let text = crate::i18n::td(format!(
                        "{} belegt · {} frei",
                        format_bytes(used),
                        format_bytes(volume.free)
                    ));
                    let label = theme::secondary(p, text).size(12.0);
                    ui.label(if share > 0.9 {
                        label.color(p.danger)
                    } else {
                        label
                    });
                });
            });
        }
        if !any {
            ui.label(theme::secondary(p, t("Kein Laufwerksbuchstabe zugewiesen")).size(12.0));
        }
    });
    let rect = inner.response.rect;
    let response = ui.interact(
        rect,
        ui.id().with(("disk-card", disk.number)),
        Sense::click(),
    );
    if response.hovered() {
        ui.painter()
            .rect_stroke(rect, p.card_radius, Stroke::new(1.5_f32, p.accent));
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.clicked()
}
