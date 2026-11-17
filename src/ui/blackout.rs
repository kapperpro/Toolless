//! Vollbildmodi über die Bildschirme: „AFK“ (schwarz), große Uhr und Spotify.
//! Je gewähltem Bildschirm entsteht ein rahmenloses Fenster; der Rechner bleibt wach.
//!
//! Beenden: F11, Esc, eine eigene Taste oder – wenn gewählt – jede Taste. Als Notausgang
//! beendet Esc, 20 Sekunden lang gehalten, immer.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, FontId, TextureHandle};

use crate::{
    app::Shared,
    i18n::td,
    platform::{self, LocalTime},
    services::{
        power::PowerState,
        spotify::{Artwork, Snapshot},
    },
};

/// Fenstertitel der Vollbildflächen – darüber findet Windows die Fenster wieder.
const TITLE: &str = "Toolless – Bildschirm";
/// So lange nach dem Start werden Eingaben ignoriert (der Klick, der gestartet hat).
const GRACE: Duration = Duration::from_millis(700);
/// So lange Esc gedrückt werden muss, damit es immer beendet.
pub const ESC_HOLD: Duration = Duration::from_secs(20);

pub const WEEKDAYS: [&str; 7] = [
    "Sonntag",
    "Montag",
    "Dienstag",
    "Mittwoch",
    "Donnerstag",
    "Freitag",
    "Samstag",
];
pub const MONTHS: [&str; 12] = [
    "Januar",
    "Februar",
    "März",
    "April",
    "Mai",
    "Juni",
    "Juli",
    "August",
    "September",
    "Oktober",
    "November",
    "Dezember",
];

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Nur schwarz.
    Afk,
    /// Schwarz mit großer Uhr auf jedem Bildschirm.
    Clock,
    /// Aktuelle Wiedergabe: Cover, Titel, Interpret und Fortschritt.
    Spotify,
}

/// Welche Tasten beenden den Modus?
#[derive(Clone, Copy)]
struct ExitKeys {
    f11: bool,
    esc: bool,
    custom: Option<egui::Key>,
    any: bool,
}

impl ExitKeys {
    fn from_shared(shared: &Shared) -> Self {
        let settings = &shared.settings;
        Self {
            f11: settings.afk_exit_f11,
            esc: settings.afk_exit_esc,
            custom: egui::Key::from_name(&settings.afk_custom_key),
            any: settings.blackout_any_key,
        }
    }

    #[allow(dead_code)]
    fn hint(&self) -> String {
        let mut keys: Vec<String> = Vec::new();
        if self.f11 {
            keys.push("F11".to_owned());
        }
        if self.esc {
            keys.push("Esc".to_owned());
        }
        if let Some(key) = self.custom {
            keys.push(key.name().to_owned());
        }
        if self.any {
            keys.push("jede Taste".to_owned());
        }
        if keys.is_empty() {
            td("Esc 20 Sekunden halten beendet".to_owned())
        } else {
            td(format!(
                "Beenden mit {} · Esc 20 Sekunden halten beendet immer",
                keys.join(", ")
            ))
        }
    }
}

/// Windows-Tastencode einer egui-Taste (nur die gängigen).
pub fn virtual_key(key: egui::Key) -> Option<i32> {
    use egui::Key;
    let name = key.name();
    let mut chars = name.chars();
    if let (Some(first), None) = (chars.next(), chars.next()) {
        if first.is_ascii_alphabetic() {
            return Some(first.to_ascii_uppercase() as i32);
        }
        if first.is_ascii_digit() {
            return Some(first as i32);
        }
    }

    if let Some(number) = name.strip_prefix('F').and_then(|n| n.parse::<i32>().ok()) {
        if (1..=12).contains(&number) {
            return Some(0x70 + number - 1);
        }
    }
    Some(match key {
        Key::Space => 0x20,
        Key::Enter => 0x0D,
        Key::Tab => 0x09,
        Key::Backspace => 0x08,
        Key::Escape => 0x1B,
        Key::ArrowLeft => 0x25,
        Key::ArrowUp => 0x26,
        Key::ArrowRight => 0x27,
        Key::ArrowDown => 0x28,
        Key::Delete => 0x2E,
        Key::Insert => 0x2D,
        Key::Home => 0x24,
        Key::End => 0x23,
        Key::PageUp => 0x21,
        Key::PageDown => 0x22,
        _ => return None,
    })
}

#[derive(Default)]
pub struct Blackout {
    active: Option<(Mode, Instant)>,
    keep_awake: PowerState,
    /// Bildschirme (Pixelrechtecke), auf denen die Fenster liegen.
    targets: Vec<[i32; 4]>,
    esc_since: Option<Instant>,
    artwork: Option<(u64, TextureHandle)>,
}

pub fn format_date(time: &LocalTime) -> String {
    let weekday = crate::i18n::t(WEEKDAYS[time.weekday as usize % 7]);
    let month = crate::i18n::t(MONTHS[(time.month as usize).clamp(1, 12) - 1]);
    format!("{weekday}, {}. {month} {}", time.day, time.year)
}

impl Blackout {
    pub fn is_active(&self) -> bool {
        self.active.is_some()
    }

    /// Braucht der aktive Modus laufend die Wiedergabedaten?
    pub fn wants_spotify(&self) -> bool {
        matches!(self.active, Some((Mode::Spotify, _)))
    }

    pub fn start(&mut self, mode: Mode, shared: &mut Shared) {
        let screens = platform::screens();
        let chosen: Vec<[i32; 4]> = shared
            .settings
            .afk_screens
            .iter()
            .filter_map(|index| screens.get(*index).copied())
            .collect();
        self.targets = if chosen.is_empty() { screens } else { chosen };
        self.active = Some((mode, Instant::now()));
        self.esc_since = None;
        if let Err(err) = self.keep_awake.set(true) {
            shared.notify(err);
        }
    }

    pub fn stop(&mut self) {
        if self.active.take().is_some() {
            let _ = self.keep_awake.set(false);
        }
        self.esc_since = None;
    }

    /// Fragt die Tastatur direkt bei Windows ab – so zählt jede Taste, auch wenn eines der
    /// Fenster gerade keinen Fokus hat. Liefert `true`, wenn der Modus enden soll.
    fn poll_exit(&mut self, age: Duration, keys: ExitKeys) -> bool {
        // Notausgang: Esc lange halten – gilt immer, auch in der Schonfrist.
        if platform::key_down(0x1B) {
            let since = *self.esc_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= ESC_HOLD {
                return true;
            }
        } else {
            self.esc_since = None;
        }
        if age < GRACE {
            return false;
        }
        (keys.f11 && platform::key_down(0x7A))
            || (keys.esc && platform::key_down(0x1B))
            || keys
                .custom
                .and_then(virtual_key)
                .is_some_and(platform::key_down)
            || (keys.any && platform::any_key_down())
    }

    fn sync_artwork(&mut self, ctx: &egui::Context, artwork: Option<&Artwork>) {
        match artwork {
            None => self.artwork = None,
            Some(artwork) => {
                if self.artwork.as_ref().map(|(id, _)| *id) != Some(artwork.id) {
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [artwork.width, artwork.height],
                        &artwork.rgba,
                    );
                    let texture =
                        ctx.load_texture("fullscreen-cover", image, egui::TextureOptions::LINEAR);
                    self.artwork = Some((artwork.id, texture));
                }
            }
        }
    }

    /// Zeichnet die Fenster; in jedem Bild aufrufen.
    pub fn show(&mut self, ctx: &egui::Context, shared: &mut Shared) {
        let Some((mode, since)) = self.active else {
            return;
        };
        let age = since.elapsed();
        let keys = ExitKeys::from_shared(shared);
        let mut close = self.poll_exit(age, keys);

        let snapshot = (mode == Mode::Spotify).then(|| shared.spotify.snapshot());
        if let Some(snapshot) = &snapshot {
            self.sync_artwork(ctx, snapshot.artwork.as_deref());
        }
        let cover = snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.artwork.as_ref())
            .map(|artwork| {
                let [r, g, b] = artwork.dominant_color();
                Color32::from_rgb(r, g, b)
            });
        let texture = self.artwork.as_ref().map(|(_, texture)| texture.clone());
        let now = platform::local_time();
        let hint: Option<String> = None;
        let esc_fraction = self.esc_since.map_or(0.0, |since| {
            (since.elapsed().as_secs_f32() / ESC_HOLD.as_secs_f32()).clamp(0.0, 1.0)
        });

        let rects: Vec<Option<[i32; 4]>> = if self.targets.is_empty() {
            vec![None]
        } else {
            self.targets.iter().map(|rect| Some(*rect)).collect()
        };
        for (index, rect) in rects.into_iter().enumerate() {
            let title = format!("{TITLE} {index}");
            let builder = egui::ViewportBuilder::default()
                .with_title(title.clone())
                .with_decorations(false)
                .with_always_on_top()
                .with_taskbar(false)
                .with_inner_size([640.0, 360.0]);
            let (snapshot, texture, hint) = (&snapshot, &texture, &hint);
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of(("toolless-blackout", index)),
                builder,
                |ctx, _class| {
                    if let Some(rect) = rect {
                        // Das erste Fenster holt anfangs den Fokus (damit Tasten ankommen).
                        platform::place_window(
                            &title,
                            rect,
                            index == 0 && age < Duration::from_secs(2),
                        );
                    }
                    ctx.set_cursor_icon(egui::CursorIcon::None);
                    paint(
                        ctx,
                        &Scene {
                            mode,
                            now,
                            snapshot: snapshot.as_ref(),
                            texture: texture.as_ref(),
                            cover,
                            hint: hint.as_deref(),
                            esc_fraction,
                        },
                    );
                    if age > GRACE {
                        close |= ctx.input(|input| {
                            let pressed = |key| input.key_pressed(key);
                            (keys.f11 && pressed(egui::Key::F11))
                                || (keys.esc && pressed(egui::Key::Escape))
                                || keys.custom.is_some_and(pressed)
                                || (keys.any
                                    && input.events.iter().any(|event| {
                                        matches!(
                                            event,
                                            egui::Event::Key { pressed: true, .. }
                                                | egui::Event::PointerButton { pressed: true, .. }
                                        )
                                    }))
                                || input.viewport().close_requested()
                        });
                    }
                    ctx.request_repaint_after(Duration::from_millis(100));
                },
            );
        }
        // Auch ohne Ereignisse regelmäßig neu zeichnen, damit die Tasten abgefragt werden.
        ctx.request_repaint_after(Duration::from_millis(100));
        if close {
            self.stop();
        }
    }
}

struct Scene<'a> {
    mode: Mode,
    now: LocalTime,
    snapshot: Option<&'a Snapshot>,
    texture: Option<&'a TextureHandle>,
    cover: Option<Color32>,
    hint: Option<&'a str>,
    esc_fraction: f32,
}

fn format_time(ms: i64) -> String {
    let total = (ms.max(0) / 1000) as u64;
    format!("{}:{:02}", total / 60, total % 60)
}

fn paint(ctx: &egui::Context, scene: &Scene) {
    let background = match (scene.mode, scene.cover) {
        (Mode::Spotify, Some(color)) => {
            Color32::from_rgb(color.r() / 7, color.g() / 7, color.b() / 7)
        }
        _ => Color32::BLACK,
    };
    egui::CentralPanel::default()
        .frame(egui::Frame::none().fill(background))
        .show(ctx, |ui| {
            let area = ui.max_rect();
            let painter = ui.painter();
            match scene.mode {
                Mode::Afk => {}
                Mode::Clock => {
                    let time = format!(
                        "{:02}:{:02}:{:02}",
                        scene.now.hour, scene.now.minute, scene.now.second
                    );
                    let size = (area.height() * 0.2).clamp(24.0, 400.0);
                    painter.text(
                        area.center(),
                        Align2::CENTER_CENTER,
                        time,
                        FontId::proportional(size),
                        Color32::from_gray(235),
                    );
                    painter.text(
                        area.center() + egui::vec2(0.0, size * 0.75),
                        Align2::CENTER_CENTER,
                        format_date(&scene.now),
                        FontId::proportional(size * 0.16),
                        Color32::from_gray(150),
                    );
                }
                Mode::Spotify => paint_spotify(ui, area, scene),
            }
            if let Some(hint) = scene.hint {
                ui.painter().text(
                    egui::pos2(area.center().x, area.bottom() - 16.0),
                    Align2::CENTER_BOTTOM,
                    hint,
                    FontId::proportional(14.0),
                    Color32::from_gray(110),
                );
            }
            if scene.esc_fraction > 0.0 {
                let width = area.width() * 0.3;
                let bar = egui::Rect::from_center_size(
                    egui::pos2(area.center().x, area.bottom() - 48.0),
                    egui::vec2(width, 6.0),
                );
                ui.painter().rect_filled(bar, 3.0, Color32::from_gray(50));
                let mut filled = bar;
                filled.set_width(width * scene.esc_fraction);
                ui.painter()
                    .rect_filled(filled, 3.0, Color32::from_gray(210));
            }
        });
}

fn paint_spotify(ui: &egui::Ui, area: egui::Rect, scene: &Scene) {
    let accent = scene.cover.unwrap_or(Color32::from_gray(220));
    let painter = ui.painter().clone();
    let playback = scene
        .snapshot
        .and_then(|snapshot| snapshot.playback.as_ref());
    let Some(playback) = playback else {
        painter.text(
            area.center(),
            Align2::CENTER_CENTER,
            crate::i18n::t("Keine Wiedergabe"),
            FontId::proportional((area.height() * 0.06).clamp(18.0, 80.0)),
            Color32::from_gray(120),
        );
        return;
    };

    let side = (area.height() * 0.55).min(area.width() * 0.4);
    let gap = side * 0.12;
    let text_width = (area.width() * 0.34).max(240.0);
    let total = side + gap + text_width;
    let left = area.center().x - total / 2.0;
    let top = area.center().y - side / 2.0;

    // Cover.
    let cover = egui::Rect::from_min_size(egui::pos2(left, top), egui::vec2(side, side));
    match scene.texture {
        Some(texture) => {
            let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            painter.image(texture.id(), cover, uv, Color32::WHITE);
        }
        None => {
            painter.rect_filled(cover, 8.0, Color32::from_gray(24));
            painter.text(
                cover.center(),
                Align2::CENTER_CENTER,
                "♪",
                FontId::proportional(side * 0.3),
                Color32::from_gray(90),
            );
        }
    }

    // Titel, Interpret, Album.
    let x = left + side + gap;
    let title_size = (side * 0.13).clamp(18.0, 64.0);
    let mut y = top + side * 0.12;
    let mut line = |text: &str, size: f32, color: Color32, bold_gap: f32| {
        let galley = painter.layout(
            text.to_owned(),
            FontId::proportional(size),
            color,
            text_width,
        );
        let height = galley.size().y;
        painter.galley(egui::pos2(x, y), galley, color);
        y += height + bold_gap;
    };
    line(
        &playback.title,
        title_size,
        Color32::WHITE,
        title_size * 0.25,
    );
    line(
        &playback.artist,
        title_size * 0.62,
        Color32::from_gray(200),
        title_size * 0.15,
    );
    if !playback.album.is_empty() {
        line(
            &playback.album,
            title_size * 0.46,
            Color32::from_gray(140),
            0.0,
        );
    }

    // Fortschritt.
    let mut progress = playback.progress_ms;
    if playback.is_playing {
        if let Some(captured) = scene.snapshot.and_then(|snapshot| snapshot.captured_at) {
            progress += captured.elapsed().as_millis() as i64;
        }
    }
    if playback.duration_ms > 0 {
        progress = progress.min(playback.duration_ms);
    }
    let bar_y = top + side * 0.88;
    let bar = egui::Rect::from_min_size(egui::pos2(x, bar_y), egui::vec2(text_width, 5.0));
    painter.rect_filled(bar, 2.5, Color32::from_gray(50));
    if playback.duration_ms > 0 {
        let mut filled = bar;
        filled.set_width(
            text_width * (progress as f32 / playback.duration_ms as f32).clamp(0.0, 1.0),
        );
        painter.rect_filled(filled, 2.5, accent);
    }
    let small = FontId::proportional((title_size * 0.4).clamp(12.0, 28.0));
    painter.text(
        egui::pos2(x, bar_y + 12.0),
        Align2::LEFT_TOP,
        format_time(progress),
        small.clone(),
        Color32::from_gray(150),
    );
    painter.text(
        egui::pos2(x + text_width, bar_y + 12.0),
        Align2::RIGHT_TOP,
        format_time(playback.duration_ms),
        small,
        Color32::from_gray(150),
    );
}
