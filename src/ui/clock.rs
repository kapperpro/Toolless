//! Uhr: Wecker, Countdown, Stoppuhr sowie „AFK“ (schwarzer Bildschirm) und die große
//! Uhr über alle Bildschirme. Beide halten den Rechner wach.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Align2, Color32, FontId, Layout, RichText, Ui};

use crate::{
    app::Shared,
    i18n::{t, td},
    platform::{self, LocalTime},
    services::{
        clock::{self, Alarm, ClockHandle, Moment},
        power::PowerState,
    },
    settings::SavedAlarm,
    theme::{self, Palette},
};

/// Fenstertitel der Vollbildfläche – darüber findet Windows das Fenster wieder.
const BLACKOUT_TITLE: &str = "Toolless – Bildschirm aus";
/// So lange nach dem Start werden Eingaben ignoriert (der Klick, der gestartet hat).
const BLACKOUT_GRACE: Duration = Duration::from_millis(700);

const WEEKDAYS: [&str; 7] = [
    "Sonntag",
    "Montag",
    "Dienstag",
    "Mittwoch",
    "Donnerstag",
    "Freitag",
    "Samstag",
];
const MONTHS: [&str; 12] = [
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
enum Blackout {
    /// Nur schwarz.
    Afk,
    /// Schwarz mit großer Uhr auf jedem Bildschirm.
    Clock,
}

pub struct ClockView {
    handle: ClockHandle,
    // Neuer Wecker.
    new_hour: u8,
    new_minute: u8,
    new_label: String,
    new_repeat: bool,
    // Countdown-Eingabe.
    hours: u32,
    minutes: u32,
    seconds: u32,
    // Stoppuhr.
    stopwatch_since: Option<Instant>,
    stopwatch_total: Duration,
    laps: Vec<Duration>,
    // Schwarzer Bildschirm.
    blackout: Option<(Blackout, Instant)>,
    keep_awake: PowerState,
    screens: Vec<[i32; 4]>,
    /// Klingelte es im letzten Bild schon? (um das Fenster nur einmal hervorzuholen)
    was_ringing: bool,
}

fn moment() -> Moment {
    let now = platform::local_time();
    Moment {
        day: now.month as u32 * 32 + now.day as u32,
        hour: now.hour,
        minute: now.minute,
    }
}

fn format_date(time: &LocalTime) -> String {
    let weekday = t(WEEKDAYS[time.weekday as usize % 7]);
    let month = t(MONTHS[(time.month as usize).clamp(1, 12) - 1]);
    format!("{weekday}, {}. {month} {}", time.day, time.year)
}

fn format_span(duration: Duration, tenths: bool) -> String {
    let total = duration.as_secs();
    let clock = format!(
        "{:02}:{:02}:{:02}",
        total / 3600,
        total % 3600 / 60,
        total % 60
    );
    if tenths {
        format!("{clock},{}", duration.subsec_millis() / 100)
    } else {
        clock
    }
}

fn section(ui: &mut Ui, p: &Palette, title: &'static str, add_contents: impl FnOnce(&mut Ui)) {
    theme::card(p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        theme::card_title(ui, t(title));
        ui.add_space(2.0);
        add_contents(ui);
    });
}

impl ClockView {
    pub fn new(saved: &[SavedAlarm], countdown_secs: u32, ctx: egui::Context) -> Self {
        let alarms = saved
            .iter()
            .map(|alarm| Alarm {
                id: alarm.id,
                hour: alarm.hour,
                minute: alarm.minute,
                label: alarm.label.clone(),
                enabled: alarm.enabled,
                repeat: alarm.repeat,
            })
            .collect();
        Self {
            handle: ClockHandle::spawn(alarms, moment, platform::beep, move || {
                ctx.request_repaint();
            }),
            new_hour: 7,
            new_minute: 0,
            new_label: String::new(),
            new_repeat: true,
            hours: countdown_secs / 3600,
            minutes: countdown_secs % 3600 / 60,
            seconds: countdown_secs % 60,
            stopwatch_since: None,
            stopwatch_total: Duration::ZERO,
            laps: Vec::new(),
            blackout: None,
            keep_awake: PowerState::default(),
            screens: Vec::new(),
            was_ringing: false,
        }
    }

    /// Schreibt die Wecker in die Einstellungen.
    fn persist_alarms(&self, shared: &mut Shared) {
        shared.settings.alarms = self.handle.read(|state| {
            state
                .alarms
                .iter()
                .map(|alarm| SavedAlarm {
                    id: alarm.id,
                    hour: alarm.hour,
                    minute: alarm.minute,
                    label: alarm.label.clone(),
                    enabled: alarm.enabled,
                    repeat: alarm.repeat,
                })
                .collect()
        });
        shared.save_settings();
    }

    /// Läuft in jedem Bild, auch wenn ein anderes Werkzeug offen ist: zeichnet die
    /// Vollbildfläche und meldet mit `true`, dass es gerade zu klingeln begonnen hat.
    pub fn background(&mut self, ctx: &egui::Context, shared: &mut Shared) -> bool {
        let (ringing, dirty) = self.handle.update(|state| {
            let dirty = state.dirty;
            state.dirty = false;
            (state.ringing.is_some(), dirty)
        });
        if dirty {
            self.persist_alarms(shared);
        }
        let started = ringing && !self.was_ringing;
        self.was_ringing = ringing;
        if started {
            // Wer geweckt wird, soll nicht auf eine schwarze Fläche schauen.
            self.stop_blackout();
        }
        self.show_blackout(ctx, shared.settings.blackout_any_key);
        started
    }

    fn start_blackout(&mut self, mode: Blackout, shared: &mut Shared) {
        self.screens = platform::screens();
        self.blackout = Some((mode, Instant::now()));
        if let Err(err) = self.keep_awake.set(true) {
            shared.notify(err);
        }
    }

    fn stop_blackout(&mut self) {
        if self.blackout.take().is_some() {
            let _ = self.keep_awake.set(false);
        }
    }

    fn show_blackout(&mut self, ctx: &egui::Context, any_key: bool) {
        let Some((mode, since)) = self.blackout else {
            return;
        };
        let age = since.elapsed();
        let origin = platform::virtual_screen();
        let screens = self.screens.clone();
        let mut close = false;
        let builder = egui::ViewportBuilder::default()
            .with_title(BLACKOUT_TITLE)
            .with_decorations(false)
            .with_always_on_top()
            .with_taskbar(false)
            .with_inner_size([640.0, 360.0]);
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("toolless-blackout"),
            builder,
            |ctx, _class| {
                // Über alle Bildschirme legen; anfangs zusätzlich den Fokus holen, damit
                // die Tasten hier ankommen.
                platform::cover_all_screens(BLACKOUT_TITLE, age < Duration::from_secs(2));
                ctx.set_cursor_icon(egui::CursorIcon::None);
                egui::CentralPanel::default()
                    .frame(egui::Frame::none().fill(Color32::BLACK))
                    .show(ctx, |ui| {
                        if mode != Blackout::Clock {
                            return;
                        }
                        let now = platform::local_time();
                        let time = format!("{:02}:{:02}:{:02}", now.hour, now.minute, now.second);
                        let date = format_date(&now);
                        let scale = ctx.pixels_per_point();
                        let area = ui.max_rect();
                        // Je Bildschirm eine Uhr; ohne Bildschirmliste eine in der Mitte.
                        let mut centers: Vec<(egui::Pos2, f32)> = screens
                            .iter()
                            .map(|[x, y, width, height]| {
                                let center = egui::pos2(
                                    ((x - origin[0]) as f32 + *width as f32 / 2.0) / scale,
                                    ((y - origin[1]) as f32 + *height as f32 / 2.0) / scale,
                                );
                                (center, *height as f32 / scale)
                            })
                            .collect();
                        if centers.is_empty() {
                            centers.push((area.center(), area.height()));
                        }
                        for (center, height) in centers {
                            let size = (height * 0.2).clamp(24.0, 400.0);
                            ui.painter().text(
                                center,
                                Align2::CENTER_CENTER,
                                &time,
                                FontId::proportional(size),
                                Color32::from_gray(235),
                            );
                            ui.painter().text(
                                center + egui::vec2(0.0, size * 0.75),
                                Align2::CENTER_CENTER,
                                &date,
                                FontId::proportional(size * 0.16),
                                Color32::from_gray(150),
                            );
                        }
                        ctx.request_repaint_after(Duration::from_millis(250));
                    });

                if age > BLACKOUT_GRACE {
                    close = ctx.input(|input| {
                        let exit_key = input.key_pressed(egui::Key::F11)
                            || input.key_pressed(egui::Key::Escape);
                        let anything = input.events.iter().any(|event| {
                            matches!(
                                event,
                                egui::Event::Key { pressed: true, .. }
                                    | egui::Event::PointerButton { pressed: true, .. }
                            )
                        });
                        exit_key || (any_key && anything) || input.viewport().close_requested()
                    });
                } else {
                    ctx.request_repaint();
                }
            },
        );
        if close {
            self.stop_blackout();
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, shared: &mut Shared) {
        let p = shared.palette;
        let now = platform::local_time();

        // ── Es klingelt ──
        if let Some(what) = self.handle.read(|state| state.ringing.clone()) {
            theme::card(&p).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("⏰  {}", td(what)))
                            .size(18.0)
                            .color(p.accent),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::primary_button(ui, &p, t("Stopp"), true).clicked() {
                            self.handle.update(|state| state.ringing = None);
                        }
                    });
                });
            });
        }

        // ── Uhrzeit und Vollbild ──
        theme::card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                let time = format!("{:02}:{:02}:{:02}", now.hour, now.minute, now.second);
                ui.label(RichText::new(time).size(64.0));
                ui.label(theme::secondary(&p, format_date(&now)).size(15.0));
            });
            ui.add_space(8.0);
            let mut start: Option<Blackout> = None;
            ui.horizontal(|ui| {
                if ui
                    .button(t("AFK – Bildschirme schwarz"))
                    .on_hover_text(t(
                        "Alle Bildschirme im Vollbild schwarz. Der Rechner bleibt wach.",
                    ))
                    .clicked()
                {
                    start = Some(Blackout::Afk);
                }
                if ui
                    .button(t("Große Uhr"))
                    .on_hover_text(t("Wie AFK, aber mit Uhr auf jedem Bildschirm."))
                    .clicked()
                {
                    start = Some(Blackout::Clock);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let mut any_key = shared.settings.blackout_any_key;
                    if ui
                        .checkbox(&mut any_key, t("Jede Taste oder Mausklick beendet"))
                        .changed()
                    {
                        shared.settings.blackout_any_key = any_key;
                        shared.save_settings();
                    }
                });
            });
            ui.label(theme::secondary(&p, t("Beenden mit F11 oder Esc.")).size(12.0));
            if let Some(mode) = start {
                self.start_blackout(mode, shared);
            }
        });

        ui.columns(2, |columns| {
            self.alarms(&mut columns[0], &p, shared);
            self.countdown(&mut columns[1], &p, shared);
            self.stopwatch(&mut columns[1], &p);
        });

        // Sekundenanzeige und Stoppuhr laufen weiter.
        let pace = if self.stopwatch_since.is_some() {
            60
        } else {
            250
        };
        ui.ctx().request_repaint_after(Duration::from_millis(pace));
    }

    fn alarms(&mut self, ui: &mut Ui, p: &Palette, shared: &mut Shared) {
        let mut changed = false;
        section(ui, p, t("Wecker"), |ui| {
            let mut alarms: Vec<Alarm> = self.handle.read(|state| state.alarms.clone());
            let mut remove: Option<u64> = None;
            if alarms.is_empty() {
                ui.label(theme::secondary(p, t("Noch kein Wecker eingerichtet.")).size(12.5));
            }
            for alarm in &mut alarms {
                ui.horizontal(|ui| {
                    if theme::toggle(ui, p, &mut alarm.enabled).changed() {
                        changed = true;
                    }
                    let time = format!("{:02}:{:02}", alarm.hour, alarm.minute);
                    let color = if alarm.enabled {
                        p.text
                    } else {
                        p.text_secondary
                    };
                    ui.label(RichText::new(time).size(20.0).color(color));
                    let kind = if alarm.repeat {
                        t("täglich")
                    } else {
                        t("einmalig")
                    };
                    let details = if alarm.label.is_empty() {
                        kind.to_owned()
                    } else {
                        format!("{} · {kind}", alarm.label)
                    };
                    ui.label(theme::secondary(p, details).size(12.5));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .small_button("✕")
                            .on_hover_text(t("Wecker löschen"))
                            .clicked()
                        {
                            remove = Some(alarm.id);
                        }
                    });
                });
            }
            if let Some(id) = remove {
                alarms.retain(|alarm| alarm.id != id);
                changed = true;
            }

            ui.separator();
            ui.horizontal(|ui| {
                ui.add(
                    egui::DragValue::new(&mut self.new_hour)
                        .range(0..=23)
                        .suffix(" h"),
                );
                ui.add(
                    egui::DragValue::new(&mut self.new_minute)
                        .range(0..=59)
                        .suffix(" min"),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_label)
                        .hint_text(t("Bezeichnung"))
                        .desired_width(130.0),
                );
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.new_repeat, t("Täglich wiederholen"));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button(t("Wecker hinzufügen")).clicked() {
                        let id = alarms.iter().map(|alarm| alarm.id).max().unwrap_or(0) + 1;
                        alarms.push(Alarm {
                            id,
                            hour: self.new_hour,
                            minute: self.new_minute,
                            label: self.new_label.trim().to_owned(),
                            enabled: true,
                            repeat: self.new_repeat,
                        });
                        alarms.sort_by_key(|alarm| (alarm.hour, alarm.minute));
                        self.new_label.clear();
                        changed = true;
                    }
                });
            });
            if changed {
                self.handle.update(|state| state.alarms = alarms);
            }
        });
        if changed {
            self.persist_alarms(shared);
        }
    }

    fn countdown(&mut self, ui: &mut Ui, p: &Palette, shared: &mut Shared) {
        section(ui, p, t("Countdown"), |ui| {
            let end = self.handle.read(|state| state.countdown_end);
            match end {
                Some(end) => {
                    let left = Duration::from_secs(end.saturating_sub(clock::unix_now()));
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new(format_span(left, false))
                                .size(36.0)
                                .color(p.accent),
                        );
                    });
                    if ui.button(t("Abbrechen")).clicked() {
                        self.handle.update(|state| state.countdown_end = None);
                    }
                }
                None => {
                    ui.horizontal(|ui| {
                        ui.add(
                            egui::DragValue::new(&mut self.hours)
                                .range(0..=99)
                                .suffix(" h"),
                        );
                        ui.add(
                            egui::DragValue::new(&mut self.minutes)
                                .range(0..=59)
                                .suffix(" min"),
                        );
                        ui.add(
                            egui::DragValue::new(&mut self.seconds)
                                .range(0..=59)
                                .suffix(" s"),
                        );
                    });
                    ui.horizontal(|ui| {
                        for (label, minutes) in
                            [("1 min", 1), ("5 min", 5), ("15 min", 15), ("30 min", 30)]
                        {
                            if ui.small_button(label).clicked() {
                                (self.hours, self.minutes, self.seconds) = (0, minutes, 0);
                            }
                        }
                    });
                    let total = self.hours * 3600 + self.minutes * 60 + self.seconds;
                    if theme::primary_button(ui, p, t("Starten"), total > 0).clicked() {
                        let end = clock::unix_now() + total as u64;
                        self.handle.update(|state| state.countdown_end = Some(end));
                        shared.settings.countdown_secs = total;
                        shared.save_settings();
                    }
                }
            }
        });
    }

    fn stopwatch(&mut self, ui: &mut Ui, p: &Palette) {
        section(ui, p, t("Stoppuhr"), |ui| {
            let running = self.stopwatch_since.is_some();
            let elapsed = self.stopwatch_total
                + self
                    .stopwatch_since
                    .map_or(Duration::ZERO, |since| since.elapsed());
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(format_span(elapsed, true)).size(36.0));
            });
            ui.horizontal(|ui| {
                let label = if running { t("Anhalten") } else { t("Starten") };
                if theme::primary_button(ui, p, label, true).clicked() {
                    match self.stopwatch_since.take() {
                        Some(since) => self.stopwatch_total += since.elapsed(),
                        None => self.stopwatch_since = Some(Instant::now()),
                    }
                }
                if ui
                    .add_enabled(running, egui::Button::new(t("Runde")))
                    .clicked()
                {
                    self.laps.push(elapsed);
                }
                let can_reset = !running && elapsed > Duration::ZERO;
                if ui
                    .add_enabled(can_reset, egui::Button::new(t("Zurücksetzen")))
                    .clicked()
                {
                    self.stopwatch_total = Duration::ZERO;
                    self.laps.clear();
                }
            });
            let mut previous = Duration::ZERO;
            for (index, lap) in self.laps.iter().enumerate() {
                let split = lap.saturating_sub(previous);
                previous = *lap;
                ui.label(
                    theme::secondary(
                        p,
                        format!(
                            "{}. {}  (+{})",
                            index + 1,
                            format_span(*lap, true),
                            format_span(split, true)
                        ),
                    )
                    .size(12.5),
                );
            }
        });
    }
}
