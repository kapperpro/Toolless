//! Rechte Seite der Registerkarte „Leistung“ – nachempfunden dem Windows-Task-Manager:
//! ein 60-Sekunden-Diagramm mit Raster, darunter große Kennzahlen links und eine Tabelle
//! mit Hardware-Angaben rechts. Der Schalter „Erweitert“ zeigt zusätzlich die einzelnen
//! Kerne, Cache-Größen, Speicherzusammensetzung und weitere Angaben.

use eframe::egui::{
    self, Align, Align2, Color32, FontId, Layout, Pos2, Rect, RichText, Sense, Shape, Stroke, Ui,
};

use super::{
    device_color, disk_value, fill_below, format_bytes, format_uptime, gap_limit, gpu_value,
    headroom, series_colors, Device, Hardware, BYTES_PER_MB,
};
use crate::{
    i18n::{t, td},
    services::{hwinfo::HwInfo, monitor::Sample},
    theme::{self, Palette},
};

/// Alles, was die Seite zum Zeichnen braucht.
pub(super) struct Info<'a> {
    pub p: &'a Palette,
    pub device: Device,
    pub hardware: &'a Hardware,
    pub hw: &'a HwInfo,
    /// Messpunkte der letzten 60 Sekunden.
    pub samples: &'a [Sample],
    pub last: &'a Sample,
    pub advanced: bool,
    /// Auslastung je Kern: (Zeitpunkt, Prozent je Kern).
    pub cores: &'a [(f64, Vec<f32>)],
}

/// Zeitfenster der Diagramme in Sekunden.
const WINDOW: f64 = 60.0;

struct Stat {
    label: &'static str,
    value: String,
    bar: Option<Color32>,
}

fn stat(label: &'static str, value: String) -> Stat {
    Stat {
        label,
        value,
        bar: None,
    }
}

fn bar_stat(label: &'static str, value: String, color: Color32) -> Stat {
    Stat {
        label,
        value,
        bar: Some(color),
    }
}

// ───────── Zahlenformate ─────────

fn one_decimal(value: f64) -> String {
    format!("{value:.1}").replace('.', ",")
}

fn gigabytes(bytes: u64) -> String {
    format!("{} GB", one_decimal(bytes as f64 / 1_073_741_824.0))
}

fn cache_size(kilobytes: u64) -> String {
    if kilobytes >= 1024 {
        format!("{} MB", one_decimal(kilobytes as f64 / 1024.0))
    } else {
        format!("{kilobytes} KB")
    }
}

/// Netzwerkgeschwindigkeit wie im Task-Manager in Bit pro Sekunde.
fn bits(bytes_per_second: f32) -> String {
    let bits = bytes_per_second as f64 * 8.0;
    if bits >= 1_000_000.0 {
        format!("{} Mbps", one_decimal(bits / 1_000_000.0))
    } else if bits >= 1000.0 {
        format!("{:.0} Kbps", bits / 1000.0)
    } else {
        format!("{bits:.0} bps")
    }
}

fn speed(bytes_per_second: f32) -> String {
    if bytes_per_second >= BYTES_PER_MB {
        format!(
            "{} MB/s",
            one_decimal(bytes_per_second as f64 / 1_048_576.0)
        )
    } else {
        format!("{:.0} KB/s", bytes_per_second / 1024.0)
    }
}

fn yes_no(value: bool) -> String {
    t(if value { "Ja" } else { "Nein" }).to_owned()
}

fn present(text: &str) -> String {
    text.to_owned()
}

// ───────── Diagramm ─────────

type Points = Vec<(f64, f32)>;

fn points(samples: &[Sample], value: &dyn Fn(&Sample) -> Option<f32>) -> Points {
    samples
        .iter()
        .filter_map(|sample| value(sample).map(|v| (sample.secs(), v)))
        .collect()
}

/// Zeichnet ein Diagramm im Stil des Task-Managers: Rand in der Linienfarbe, Raster, das mit
/// der Zeit wandert, Linie mit zarter Füllung.
fn draw_graph(
    ui: &Ui,
    p: &Palette,
    rect: Rect,
    series: &[(Color32, Points)],
    max: f32,
    now: f64,
    gap: f64,
) {
    let painter = ui.painter_at(rect.expand(1.0));
    painter.rect_filled(rect, 0.0, p.field_bg);
    let accent = series.first().map_or(p.accent, |(color, _)| *color);

    // Raster
    let columns = if rect.width() < 300.0 { 5 } else { 10 };
    let rows = if rect.height() < 120.0 { 4 } else { 10 };
    let grid = Stroke::new(1.0_f32, accent.gamma_multiply(0.22));
    for row in 1..rows {
        let y = rect.top() + rect.height() * row as f32 / rows as f32;
        painter.hline(rect.x_range(), y, grid);
    }
    let step = WINDOW / columns as f64;
    let offset = now.rem_euclid(step);
    for column in 0..=columns {
        let x = rect.right() - ((offset + column as f64 * step) / WINDOW) as f32 * rect.width();
        if x > rect.left() && x < rect.right() {
            painter.vline(x, rect.y_range(), grid);
        }
    }

    // Linien
    let start = now - WINDOW;
    let x_of =
        |time: f64| rect.left() + (((time - start) / WINDOW) as f32).clamp(0.0, 1.0) * rect.width();
    let y_of = |value: f32| {
        rect.bottom() - 1.0 - (value / max.max(0.001)).clamp(0.0, 1.0) * (rect.height() - 2.0)
    };
    for (color, values) in series {
        let mut segment: Vec<Pos2> = Vec::new();
        let mut previous = 0.0f64;
        let flush = |segment: &mut Vec<Pos2>| {
            if segment.len() > 1 {
                fill_below(&painter, segment, rect.bottom(), color.gamma_multiply(0.2));
                painter.add(Shape::line(
                    std::mem::take(segment),
                    Stroke::new(1.5_f32, *color),
                ));
            } else {
                segment.clear();
            }
        };
        for (time, value) in values {
            if *time < start - gap {
                continue;
            }
            if !segment.is_empty() && *time - previous > gap {
                flush(&mut segment);
            }
            segment.push(Pos2::new(x_of(*time), y_of(*value)));
            previous = *time;
        }
        flush(&mut segment);
    }

    painter.rect_stroke(rect, 0.0, Stroke::new(1.0_f32, accent.gamma_multiply(0.85)));
}

/// Beschriftung über dem Diagramm: links, was es zeigt, rechts der Höchstwert.
fn caption(ui: &mut Ui, p: &Palette, left: &str, right: &str) {
    ui.horizontal(|ui| {
        ui.label(theme::secondary(p, left).size(12.0));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(theme::secondary(p, right).size(12.0));
        });
    });
}

/// Ein Diagramm mit Beschriftungen oben und unten.
fn graph(
    ui: &mut Ui,
    p: &Palette,
    title: &str,
    top_right: &str,
    series: &[(Color32, Points)],
    max: f32,
    now: f64,
    gap: f64,
    height: f32,
) {
    caption(ui, p, title, top_right);
    let size = egui::vec2(ui.available_width(), height);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        draw_graph(ui, p, rect, series, max, now, gap);
    }
    caption(ui, p, t("60 Sekunden"), "0");
}

/// Raster mit einem kleinen Diagramm je logischem Prozessor.
fn core_grid(ui: &mut Ui, info: &Info, height: f32, now: f64) {
    let p = info.p;
    let count = info
        .cores
        .last()
        .map_or(0, |(_, values)| values.len())
        .max(info.hardware.cores.len());
    if count == 0 {
        ui.label(theme::secondary(p, t("Noch keine Messwerte.")));
        return;
    }
    let columns = (count as f32).sqrt().ceil().max(1.0) as usize;
    let rows = count.div_ceil(columns);
    let gap = 6.0;
    let width = ((ui.available_width() - gap * (columns - 1) as f32) / columns as f32).max(20.0);
    let cell_height = ((height - gap * (rows - 1) as f32) / rows as f32).max(24.0);
    let (area, _) = ui.allocate_exact_size(
        egui::vec2(
            ui.available_width(),
            cell_height * rows as f32 + gap * (rows - 1) as f32,
        ),
        Sense::hover(),
    );
    let color = device_color(p, Device::Cpu);
    for index in 0..count {
        let rect = Rect::from_min_size(
            area.min
                + egui::vec2(
                    (index % columns) as f32 * (width + gap),
                    (index / columns) as f32 * (cell_height + gap),
                ),
            egui::vec2(width, cell_height),
        );
        let values: Points = info
            .cores
            .iter()
            .filter_map(|(time, usage)| usage.get(index).map(|value| (*time, *value)))
            .collect();
        let latest = values.last().map_or(0.0, |(_, value)| *value);
        draw_graph(ui, p, rect, &[(color, values)], 100.0, now, 5.0);
        ui.painter_at(rect).text(
            rect.left_top() + egui::vec2(4.0, 2.0),
            Align2::LEFT_TOP,
            format!("{index}  ·  {latest:.0} %"),
            FontId::proportional(10.5),
            p.text_secondary,
        );
    }
}

// ───────── Kennzahlen ─────────

fn draw_stat(ui: &mut Ui, p: &Palette, item: &Stat) {
    ui.horizontal(|ui| {
        // Nie umbrechen: sonst zerfallen die Werte in schmalen Zellen Buchstabe für Buchstabe.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        if let Some(color) = item.bar {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(3.0, 46.0), Sense::hover());
            ui.painter().rect_filled(rect, 0.0, color);
            ui.add_space(2.0);
        }
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(theme::secondary(p, item.label).size(12.0));
            ui.label(RichText::new(item.value.as_str()).size(24.0));
        });
    });
}

fn draw_stats(ui: &mut Ui, p: &Palette, rows: &[Vec<Stat>]) {
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
    egui::Grid::new("perf-stats")
        .spacing(egui::vec2(34.0, 10.0))
        .show(ui, |ui| {
            for row in rows {
                for item in row {
                    draw_stat(ui, p, item);
                }
                ui.end_row();
            }
        });
}

fn draw_facts(ui: &mut Ui, p: &Palette, rows: &[(&'static str, String)]) {
    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
    egui::Grid::new("perf-facts")
        .spacing(egui::vec2(16.0, 5.0))
        .show(ui, |ui| {
            for (label, value) in rows {
                ui.label(theme::secondary(p, format!("{}:", t(label))).size(12.5));
                ui.label(RichText::new(value.as_str()).size(12.5));
                ui.end_row();
            }
        });
}

// ───────── Inhalt je Gerät ─────────

type Rows = Vec<Vec<Stat>>;
type Facts = Vec<(&'static str, String)>;

fn cpu_details(info: &Info) -> (Rows, Facts) {
    let (hardware, hw, last) = (info.hardware, info.hw, info.last);
    let stats = &hardware.stats;
    let mut top = vec![stat("Auslastung", format!("{:.0} %", last.cpu))];
    if hardware.cpu_mhz > 0 {
        top.push(stat(
            "Geschwindigkeit",
            format!("{:.2} GHz", hardware.cpu_mhz as f32 / 1000.0).replace('.', ","),
        ));
    }
    let processes = if stats.processes > 0 {
        stats.processes as usize
    } else {
        hardware.process_count
    };
    let mut middle = vec![stat("Prozesse", processes.to_string())];
    if stats.threads > 0 {
        middle.push(stat("Threads", stats.threads.to_string()));
        middle.push(stat("Handles", stats.handles.to_string()));
    }
    let rows = vec![
        top,
        middle,
        vec![stat("Betriebszeit", format_uptime(hardware.uptime))],
    ];

    let mut facts: Facts = Vec::new();
    if hw.cpu.base_mhz > 0 {
        facts.push((
            "Basisgeschwindigkeit",
            format!("{:.2} GHz", hw.cpu.base_mhz as f64 / 1000.0).replace('.', ","),
        ));
    }
    if hw.cpu.sockets > 0 {
        facts.push(("Sockets", hw.cpu.sockets.to_string()));
    }
    if hw.cpu.cores > 0 {
        facts.push(("Kerne", hw.cpu.cores.to_string()));
    }
    let logical = if hw.cpu.logical > 0 {
        hw.cpu.logical as usize
    } else {
        hardware.cpu_cores
    };
    facts.push(("Logische Prozessoren", logical.to_string()));
    if info.advanced {
        if let Some(enabled) = hw.cpu.virtualization {
            facts.push((
                "Virtualisierung",
                t(if enabled { "Aktiviert" } else { "Deaktiviert" }).to_owned(),
            ));
        }
        for (label, kilobytes) in [
            ("L1-Cache", hw.cpu.l1_kb),
            ("L2-Cache", hw.cpu.l2_kb),
            ("L3-Cache", hw.cpu.l3_kb),
        ] {
            if kilobytes > 0 {
                facts.push((label, cache_size(kilobytes)));
            }
        }
        if let Some(temperature) = last.cpu_temp {
            facts.push(("Temperatur", format!("{temperature:.0} °C")));
        }
    }
    (rows, facts)
}

/// Belegter Speicher: Gesamt minus verfügbar, wie im Task-Manager.
fn memory_in_use(hardware: &Hardware) -> u64 {
    let stats = &hardware.stats;
    if stats.visible_total > 0 && stats.available > 0 {
        stats.visible_total.saturating_sub(stats.available)
    } else {
        hardware.used_ram
    }
}

fn memory_details(info: &Info) -> (Rows, Facts) {
    let (hardware, hw) = (info.hardware, info.hw);
    let stats = &hardware.stats;
    let in_use = memory_in_use(hardware);
    let available = if stats.available > 0 {
        stats.available
    } else {
        hardware.total_ram.saturating_sub(hardware.used_ram)
    };
    let used_text = if stats.compressed > 0 {
        format!("{} ({})", gigabytes(in_use), gigabytes(stats.compressed))
    } else {
        gigabytes(in_use)
    };
    let mut rows: Rows = vec![vec![
        stat("In Verwendung (komprimiert)", used_text),
        stat("Verfügbar", gigabytes(available)),
    ]];
    if stats.commit_limit > 0 {
        rows.push(vec![
            stat(
                "Zugesichert",
                format!(
                    "{}/{}",
                    one_decimal(stats.commit_total as f64 / 1_073_741_824.0),
                    gigabytes(stats.commit_limit)
                ),
            ),
            stat("Zwischengespeichert", gigabytes(stats.cached)),
        ]);
        if info.advanced {
            rows.push(vec![
                stat("Ausgelagerter Pool", gigabytes(stats.paged_pool)),
                stat("Nicht ausgelagerter Pool", gigabytes(stats.nonpaged_pool)),
            ]);
        }
    }

    let mut facts: Facts = Vec::new();
    if hw.memory.speed > 0 {
        facts.push(("Geschwindigkeit", format!("{} MT/s", hw.memory.speed)));
    }
    if hw.memory.slots_total > 0 {
        facts.push((
            "Verwendete Steckplätze",
            td(format!(
                "{} von {}",
                hw.memory.slots_used, hw.memory.slots_total
            )),
        ));
    }
    if !hw.memory.form_factor.is_empty() {
        facts.push(("Formfaktor", present(&hw.memory.form_factor)));
    }
    if hw.memory.installed > stats.visible_total && stats.visible_total > 0 {
        let reserved = hw.memory.installed - stats.visible_total;
        facts.push((
            "Für Hardware reserviert",
            format!("{:.0} MB", reserved as f64 / 1_048_576.0),
        ));
    }
    if info.advanced {
        facts.push(("Gesamt", format_bytes(hardware.total_ram)));
        if hardware.swap_total > 0 {
            facts.push((
                "Auslagerungsdatei",
                format!(
                    "{} / {}",
                    format_bytes(hardware.swap_used),
                    format_bytes(hardware.swap_total)
                ),
            ));
        }
    }
    (rows, facts)
}

fn disk_details(info: &Info, index: usize) -> (Rows, Facts) {
    let (hardware, hw, last) = (info.hardware, info.hw, info.last);
    let (_, orange, green) = series_colors(info.p);
    let latency = hardware.disk_latency.get(index).copied().unwrap_or(0.0);
    let rows = vec![
        vec![
            stat(
                "Aktive Zeit",
                format!("{:.0} %", disk_value(last, index, 2).unwrap_or(0.0)),
            ),
            stat(
                "Durchschnittliche Antwortzeit",
                format!("{latency:.1} ms").replace('.', ","),
            ),
        ],
        vec![
            bar_stat(
                "Lesegeschwindigkeit",
                speed(disk_value(last, index, 0).unwrap_or(0.0)),
                green,
            ),
            bar_stat(
                "Schreibgeschwindigkeit",
                speed(disk_value(last, index, 1).unwrap_or(0.0)),
                orange,
            ),
        ],
    ];

    let mut facts: Facts = Vec::new();
    if let Some(disk) = hardware.physical_disk(index) {
        facts.push(("Kapazität", format_bytes(disk.size)));
        let formatted: u64 = disk
            .partitions
            .iter()
            .filter(|part| !part.unallocated)
            .map(|part| part.size)
            .sum();
        if formatted > 0 {
            facts.push(("Formatiert", format_bytes(formatted)));
        }
        let has_letter = |letter: &str| {
            !letter.is_empty()
                && disk
                    .partitions
                    .iter()
                    .any(|part| part.letter.eq_ignore_ascii_case(letter))
        };
        if !hw.system_drive.is_empty() {
            facts.push((
                "Systemdatenträger",
                yes_no(has_letter(hw.system_drive.as_str())),
            ));
        }
        if hw.loaded {
            let page = hw
                .page_files
                .iter()
                .any(|letter| has_letter(letter.as_str()));
            facts.push(("Auslagerungsdatei", yes_no(page)));
        }
        let media = hw
            .disks
            .iter()
            .find(|kind| kind.number as usize == index)
            .map(|kind| kind.media.as_str())
            .unwrap_or("");
        let bus = if disk.bus.is_empty() {
            hw.disks
                .iter()
                .find(|kind| kind.number as usize == index)
                .map(|kind| kind.bus.clone())
                .unwrap_or_default()
        } else {
            disk.bus.clone()
        };
        let kind = match (media.is_empty(), bus.is_empty()) {
            (false, false) => format!("{media} ({bus})"),
            (false, true) => media.to_owned(),
            (true, false) => bus,
            (true, true) => String::new(),
        };
        if !kind.is_empty() {
            facts.push(("Typ", kind));
        }
        if info.advanced {
            if !disk.model.is_empty() {
                facts.push(("Modell", present(&disk.model)));
            }
            if !disk.style.is_empty() {
                facts.push(("Partitionsstil", present(&disk.style)));
            }
            facts.push(("Partitionen", disk.partitions.len().to_string()));
        }
    }
    (rows, facts)
}

/// Die Netzwerkschnittstelle, die gerade verbunden ist und eine Adresse hat.
fn main_network(hw: &HwInfo) -> Option<&crate::services::hwinfo::NetworkInfo> {
    hw.networks
        .iter()
        .find(|net| net.up && !net.ipv4.is_empty() && !net.kind.is_empty())
        .or_else(|| {
            hw.networks
                .iter()
                .find(|net| net.up && !net.ipv4.is_empty())
        })
        .or_else(|| hw.networks.first())
}

fn network_details(info: &Info) -> (Rows, Facts) {
    let (hw, last) = (info.hw, info.last);
    let (_, orange, green) = series_colors(info.p);
    let rows = vec![vec![
        bar_stat("Senden", bits(last.net_up), orange),
        bar_stat("Empfangen", bits(last.net_down), green),
    ]];
    let mut facts: Facts = Vec::new();
    if let Some(net) = main_network(hw) {
        facts.push(("Adaptername", present(&net.name)));
        if !net.dns_suffix.is_empty() {
            facts.push(("DNS-Name", present(&net.dns_suffix)));
        }
        if !net.kind.is_empty() {
            facts.push(("Verbindungstyp", present(&net.kind)));
        }
        if let Some(address) = net.ipv4.first() {
            facts.push(("IPv4-Adresse", present(address)));
        }
        if let Some(address) = net.ipv6.first() {
            facts.push(("IPv6-Adresse", present(address)));
        }
        if info.advanced && !net.description.is_empty() {
            facts.push(("Beschreibung", present(&net.description)));
        }
    }
    (rows, facts)
}

/// Technische Angaben zur Grafikkarte `index` (Treiber, Speicher).
fn gpu_details_of<'a>(
    info: &'a Info,
    index: usize,
) -> Option<&'a crate::services::hwinfo::GpuDetails> {
    let name = info.hardware.gpus.get(index).map(|gpu| gpu.name.as_str());
    name.and_then(|name| {
        info.hw
            .gpus
            .iter()
            .find(|gpu| gpu.name.eq_ignore_ascii_case(name))
    })
    .or_else(|| info.hw.gpus.get(index))
}

fn gpu_details(info: &Info, index: usize) -> (Rows, Facts) {
    let (hardware, last) = (info.hardware, info.last);
    let gpu = hardware.gpus.get(index);
    let details = gpu_details_of(info, index);
    let usage = gpu_value(last, index, 0).map_or_else(|| "–".to_owned(), |u| format!("{u:.0} %"));

    let dedicated_used = gpu.and_then(|gpu| gpu.memory_used);
    let shared_used = gpu.and_then(|gpu| gpu.shared_used);
    let dedicated_total = details.map_or(0, |gpu| gpu.dedicated);
    let shared_total = hardware.total_ram / 2;
    let of_total = |used: Option<u64>, total: u64| match (used, total) {
        (Some(used), total) if total > 0 => format!(
            "{}/{}",
            one_decimal(used as f64 / 1_073_741_824.0),
            gigabytes(total)
        ),
        (Some(used), _) => gigabytes(used),
        _ => "–".to_owned(),
    };

    let mut rows: Rows = vec![vec![stat("Auslastung", usage)]];
    if dedicated_used.is_some() || shared_used.is_some() {
        let used = dedicated_used.unwrap_or(0) + shared_used.unwrap_or(0);
        let total = if dedicated_total > 0 {
            dedicated_total + shared_total
        } else {
            0
        };
        rows[0].push(stat("GPU-Speicher", of_total(Some(used), total)));
        rows.push(vec![
            stat(
                "Dedizierter GPU-Speicher",
                of_total(dedicated_used, dedicated_total),
            ),
            stat(
                "Gemeinsamer GPU-Speicher",
                of_total(shared_used, shared_total),
            ),
        ]);
    }
    if let Some(temperature) = gpu_value(last, index, 1) {
        rows.push(vec![stat("Temperatur", format!("{temperature:.0} °C"))]);
    }

    let mut facts: Facts = Vec::new();
    if let Some(details) = details {
        if !details.driver.is_empty() {
            facts.push(("Treiberversion", present(&details.driver)));
        }
        if !details.driver_date.is_empty() {
            facts.push(("Treiberdatum", present(&details.driver_date)));
        }
        if !details.directx.is_empty() {
            facts.push(("DirectX-Version", present(&details.directx)));
        }
        if !details.location.is_empty() {
            facts.push(("Physischer Standort", present(&details.location)));
        }
        if info.advanced {
            facts.push(("Modell", present(&details.name)));
        }
    }
    (rows, facts)
}

fn details(info: &Info) -> (Rows, Facts) {
    match info.device {
        Device::Cpu => cpu_details(info),
        Device::Memory => memory_details(info),
        Device::Disk(index) => disk_details(info, index),
        Device::Network => network_details(info),
        Device::Gpu(index) => gpu_details(info, index),
    }
}

// ───────── Seite ─────────

/// Anzahl der Diagramme (jeweils mit Beschriftung), damit die Höhe ohne Scrollen passt.
fn graph_count(info: &Info) -> usize {
    let has_temperature = |value: &dyn Fn(&Sample) -> Option<f32>| {
        info.samples.iter().any(|sample| value(sample).is_some())
    };
    match info.device {
        Device::Cpu => 1 + usize::from(info.advanced && has_temperature(&|s| s.cpu_temp)),
        Device::Gpu(index) => {
            1 + usize::from(info.advanced && has_temperature(&|s| gpu_value(s, index, 1)))
        }
        Device::Disk(_) => 2,
        Device::Memory | Device::Network => 1,
    }
}

fn networks_len(info: &Info) -> usize {
    info.hardware.networks.len().min(6)
}

/// Zeichnet Diagramme, Kennzahlen und Angaben des gewählten Geräts.
pub(super) fn body(ui: &mut Ui, info: &Info) {
    let p = info.p;
    let now = info.samples.last().map_or(0.0, |sample| sample.secs());
    let gap = gap_limit(info.samples).max(3.0);
    let (rows, facts) = details(info);

    // Höhe der Diagramme: Rest des Fensters minus Kennzahlen, damit nichts scrollt.
    let stats_height = (rows.len() as f32 * 64.0).max(facts.len() as f32 * 23.0) + 40.0;
    let extra = match (info.device, info.advanced) {
        (Device::Memory, true) => 54.0,
        (Device::Network, true) => 24.0 * (info.hardware.networks.len().min(6) + 1) as f32 + 40.0,
        _ => 0.0,
    };
    let charts = graph_count(info);
    // Die Höhe wird einmal berechnet und gemerkt. Würde sie in jedem Bild neu aus der
    // Scrollposition abgeleitet, wüchsen die Diagramme beim Scrollen.
    let (device_kind, device_index) = match info.device {
        Device::Cpu => (0, 0),
        Device::Memory => (1, 0),
        Device::Disk(index) => (2, index),
        Device::Network => (3, 0),
        Device::Gpu(index) => (4, index),
    };
    let key = egui::Id::new((
        "perf-height",
        device_kind,
        device_index,
        info.advanced,
        rows.len(),
        facts.len(),
        networks_len(info),
        ui.clip_rect().height().round() as i32,
    ));
    let remembered = ui.data(|data| data.get_temp::<(f32, u8)>(key));
    let (height, round) = remembered.unwrap_or_else(|| {
        let remaining = (ui.clip_rect().bottom() - ui.next_widget_position().y).max(0.0);
        let height = ((remaining - stats_height - extra - charts as f32 * 58.0 - 24.0)
            / charts as f32)
            .clamp(50.0, 520.0);
        ui.data_mut(|data| data.insert_temp(key, (height, 0u8)));
        (height, 0)
    });

    let (_, orange, green) = series_colors(info.p);

    let color = device_color(p, info.device);
    let samples = info.samples;
    match info.device {
        Device::Cpu => {
            if info.advanced {
                caption(
                    ui,
                    p,
                    t("% Auslastung über 60 Sekunden – logische Prozessoren"),
                    "100 %",
                );
                core_grid(ui, info, height, now);
            } else {
                let series = [(color, points(samples, &|s| Some(s.cpu)))];
                graph(
                    ui,
                    p,
                    t("% Auslastung über 60 Sekunden"),
                    "100 %",
                    &series,
                    100.0,
                    now,
                    gap,
                    height,
                );
            }
            if info.advanced && graph_count(info) > 1 {
                let series = [(orange, points(samples, &|s| s.cpu_temp))];
                graph(
                    ui,
                    p,
                    t("Temperatur über 60 Sekunden"),
                    "100 °C",
                    &series,
                    100.0,
                    now,
                    gap,
                    height,
                );
            }
        }
        Device::Memory => {
            let total = if info.hardware.stats.visible_total > 0 {
                info.hardware.stats.visible_total
            } else {
                info.hardware.total_ram
            };
            let series = [(color, points(samples, &|s| Some(s.ram)))];
            graph(
                ui,
                p,
                t("Speicherauslastung"),
                &gigabytes(total),
                &series,
                100.0,
                now,
                gap,
                height,
            );
            if info.advanced {
                composition(ui, info);
            }
        }
        Device::Disk(index) => {
            let series = [(color, points(samples, &|s| disk_value(s, index, 2)))];
            graph(
                ui,
                p,
                t("Aktive Zeit"),
                "100 %",
                &series,
                100.0,
                now,
                gap,
                height,
            );
            let max = headroom(
                samples,
                |s| {
                    disk_value(s, index, 0)
                        .unwrap_or(0.0)
                        .max(disk_value(s, index, 1).unwrap_or(0.0))
                },
                BYTES_PER_MB,
            );
            let series = [
                (green, points(samples, &|s| disk_value(s, index, 0))),
                (orange, points(samples, &|s| disk_value(s, index, 1))),
            ];
            graph(
                ui,
                p,
                t("Datenübertragungsrate"),
                &speed(max),
                &series,
                max,
                now,
                gap,
                height,
            );
        }
        Device::Network => {
            let max = headroom(samples, |s| s.net_down.max(s.net_up), 12_500.0);
            let series = [
                (green, points(samples, &|s| Some(s.net_down))),
                (orange, points(samples, &|s| Some(s.net_up))),
            ];
            graph(
                ui,
                p,
                t("Durchsatz"),
                &bits(max),
                &series,
                max,
                now,
                gap,
                height,
            );
        }
        Device::Gpu(index) => {
            let series = [(color, points(samples, &|s| gpu_value(s, index, 0)))];
            graph(
                ui,
                p,
                t("% Auslastung über 60 Sekunden"),
                "100 %",
                &series,
                100.0,
                now,
                gap,
                height,
            );
            if info.advanced && graph_count(info) > 1 {
                let series = [(orange, points(samples, &|s| gpu_value(s, index, 1)))];
                graph(
                    ui,
                    p,
                    t("Temperatur über 60 Sekunden"),
                    "100 °C",
                    &series,
                    100.0,
                    now,
                    gap,
                    height,
                );
            }
        }
    }

    ui.add_space(8.0);
    ui.horizontal_top(|ui| {
        let half = (ui.available_width() * 0.5 - 12.0).max(200.0);
        ui.vertical(|ui| {
            ui.set_width(half);
            draw_stats(ui, p, &rows);
        });
        ui.vertical(|ui| {
            if !info.hw.loaded {
                ui.label(theme::secondary(p, t("Hardware-Angaben werden gelesen …")).size(12.5));
            }
            draw_facts(ui, p, &facts);
        });
    });

    if info.advanced && info.device == Device::Network {
        adapters(ui, info);
    }

    // Selbstkorrektur: Die Schätzung der Höhe stimmt selten genau. In den ersten Bildern wird
    // gemessen, ob der Inhalt über den sichtbaren Bereich hinausragt oder Platz übrig bleibt,
    // und die Diagrammhöhe entsprechend angepasst. Danach bleibt sie fest.
    if round < 10 {
        let overflow = ui.min_rect().bottom() - ui.clip_rect().bottom() + 6.0;
        let mut corrected = height;
        if overflow > 0.5 {
            corrected = (height - overflow / charts as f32).max(50.0);
        } else if -overflow > 20.0 && height < 520.0 {
            corrected = (height + (-overflow - 8.0) / charts as f32).min(520.0);
        }
        let settled = (corrected - height).abs() < 1.0;
        ui.data_mut(|data| {
            data.insert_temp(key, (corrected, if settled { 10u8 } else { round + 1 }))
        });
        if !settled {
            ui.ctx().request_repaint();
        }
    }
}

/// Balken „Speicherzusammensetzung“: belegt, zwischengespeichert, frei.
fn composition(ui: &mut Ui, info: &Info) {
    let p = info.p;
    let hardware = info.hardware;
    let stats = &hardware.stats;
    let visible = if stats.visible_total > 0 {
        stats.visible_total
    } else {
        hardware.total_ram
    };
    let total = visible.max(1);
    let in_use = memory_in_use(hardware).min(total);
    let available = total - in_use;
    let cached = stats.cached.min(available);
    let free = available - cached;

    ui.add_space(6.0);
    ui.label(theme::secondary(p, t("Speicherzusammensetzung")).size(12.0));
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 20.0), Sense::hover());
    let (accent, _, _) = series_colors(p);
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, p.field_bg);
    let mut x = rect.left();
    for (bytes, color) in [(in_use, accent), (cached, accent.gamma_multiply(0.4))] {
        let width = rect.width() * bytes as f32 / total as f32;
        painter.rect_filled(
            Rect::from_min_size(Pos2::new(x, rect.top()), egui::vec2(width, rect.height())),
            0.0,
            color,
        );
        x += width;
    }
    painter.rect_stroke(rect, 0.0, Stroke::new(1.0_f32, accent.gamma_multiply(0.85)));
    ui.label(
        theme::secondary(
            p,
            td(format!(
                "In Verwendung {} · Zwischengespeichert {} · Frei {}",
                gigabytes(in_use),
                gigabytes(cached),
                gigabytes(free)
            )),
        )
        .size(12.0),
    );
}

/// Durchsatz je Netzwerkschnittstelle (Erweitert).
fn adapters(ui: &mut Ui, info: &Info) {
    let p = info.p;
    if info.hardware.networks.is_empty() {
        return;
    }
    ui.add_space(8.0);
    egui::Grid::new("perf-adapters")
        .num_columns(3)
        .striped(true)
        .spacing(egui::vec2(24.0, 4.0))
        .show(ui, |ui| {
            for title in ["Adapter", "Empfangen", "Gesendet"] {
                ui.label(theme::secondary(p, title).size(12.0));
            }
            ui.end_row();
            for network in info.hardware.networks.iter().take(6) {
                ui.label(RichText::new(network.name.as_str()).size(12.5));
                ui.label(RichText::new(bits(network.down)).size(12.5));
                ui.label(RichText::new(bits(network.up)).size(12.5));
                ui.end_row();
            }
        });
}
