//! Uhr: Wecker, mehrere Countdowns und Stoppuhren sowie die Vollbildmodi „AFK“, „Große Uhr“
//! und Spotify (siehe `blackout`). Wecker und Countdowns klingeln auch im Hintergrund.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Align2, Layout, RichText, Ui};

use crate::{
    app::Shared,
    i18n::{t, td},
    platform,
    services::clock::{self, Alarm, ClockHandle, Countdown, Moment},
    settings::{SavedAlarm, SavedCountdown},
    theme::{self, Palette},
    ui::blackout::{self, format_date, Mode},
};

/// Eine Stoppuhr; läuft nur, solange die App läuft.
struct Stopwatch {
    id: u64,
    label: String,
    since: Option<Instant>,
    total: Duration,
    laps: Vec<Duration>,
}

impl Stopwatch {
    fn elapsed(&self) -> Duration {
        self.total + self.since.map_or(Duration::ZERO, |since| since.elapsed())
    }
}

/// Eingabefenster zum Hinzufügen.
enum Dialog {
    Alarm {
        hour: u8,
        minute: u8,
        label: String,
        repeat: bool,
    },
    Countdown {
        hours: u32,
        minutes: u32,
        seconds: u32,
        label: String,
    },
}

pub struct ClockView {
    handle: ClockHandle,
    dialog: Option<Dialog>,
    stopwatches: Vec<Stopwatch>,
    next_stopwatch: u64,
    /// Wartet auf einen Tastendruck für die eigene Beenden-Taste.
    capturing_key: bool,
    /// Klingelte es im letzten Bild schon? (um das Fenster nur einmal hervorzuholen)
    was_ringing: bool,
}

fn moment() -> Moment {
    let now = platform::local_time();
    Moment {
        day: now.year as u32 * 512 + now.month as u32 * 32 + now.day as u32,
        hour: now.hour,
        minute: now.minute,
    }
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

/// Karte mit Titel und einem „+“-Knopf rechts oben. Liefert `true`, wenn er gedrückt wurde.
fn section(
    ui: &mut Ui,
    p: &Palette,
    title: &'static str,
    add_button: Option<&'static str>,
    add_contents: impl FnOnce(&mut Ui),
) -> bool {
    let mut add = false;
    theme::card(p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            theme::card_title(ui, t(title));
            if let Some(label) = add_button {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button(t(label)).clicked() {
                        add = true;
                    }
                });
            }
        });
        ui.add_space(2.0);
        add_contents(ui);
    });
    add
}

/// Auswahl der Bildschirme für die Vollbildmodi (leer = alle).
pub fn screen_picker(ui: &mut Ui, shared: &mut Shared) {
    let screens = platform::screens();
    let mut chosen = shared.settings.afk_screens.clone();
    chosen.retain(|index| *index < screens.len().max(1));
    let text = if chosen.is_empty() {
        t("Alle Bildschirme").to_owned()
    } else {
        let numbers: Vec<String> = chosen.iter().map(|index| (index + 1).to_string()).collect();
        td(format!("Bildschirm {}", numbers.join(", ")))
    };
    let mut changed = false;
    egui::ComboBox::from_id_salt("blackout_screens")
        .selected_text(text)
        .show_ui(ui, |ui| {
            let mut all = chosen.is_empty();
            if ui.checkbox(&mut all, t("Alle Bildschirme")).changed() {
                chosen = if all { Vec::new() } else { vec![0] };
                changed = true;
            }
            for (index, [_, _, width, height]) in screens.iter().enumerate() {
                let mut on = chosen.contains(&index);
                let label = td(format!("Bildschirm {} · {width}×{height}", index + 1));
                if ui.checkbox(&mut on, label).changed() {
                    if on {
                        chosen.push(index);
                        chosen.sort_unstable();
                    } else {
                        chosen.retain(|other| *other != index);
                    }
                    changed = true;
                }
            }
        });
    if changed {
        shared.settings.afk_screens = chosen;
        shared.save_settings();
    }
}

impl ClockView {
    pub fn new(
        saved_alarms: &[SavedAlarm],
        saved_countdowns: &[SavedCountdown],
        ctx: egui::Context,
    ) -> Self {
        let alarms = saved_alarms
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
        let countdowns = saved_countdowns
            .iter()
            .map(|countdown| Countdown {
                id: countdown.id,
                label: countdown.label.clone(),
                secs: countdown.secs,
                end: None,
            })
            .collect();
        Self {
            handle: ClockHandle::spawn(alarms, countdowns, moment, platform::beep, move || {
                ctx.request_repaint();
            }),
            dialog: None,
            stopwatches: Vec::new(),
            next_stopwatch: 1,
            capturing_key: false,
            was_ringing: false,
        }
    }

    /// Schreibt Wecker und Countdowns in die Einstellungen.
    fn persist(&self, shared: &mut Shared) {
        let (alarms, countdowns) = self.handle.read(|state| {
            (
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
                    .collect(),
                state
                    .countdowns
                    .iter()
                    .map(|countdown| SavedCountdown {
                        id: countdown.id,
                        label: countdown.label.clone(),
                        secs: countdown.secs,
                    })
                    .collect(),
            )
        });
        shared.settings.alarms = alarms;
        shared.settings.countdowns = countdowns;
        shared.save_settings();
    }

    /// Läuft in jedem Bild, auch wenn ein anderes Werkzeug offen ist; meldet mit `true`,
    /// dass es gerade zu klingeln begonnen hat.
    pub fn background(&mut self, shared: &mut Shared) -> bool {
        let (ringing, dirty) = self.handle.update(|state| {
            let dirty = state.dirty;
            state.dirty = false;
            (state.ringing.is_some(), dirty)
        });
        if dirty {
            self.persist(shared);
        }
        let started = ringing && !self.was_ringing;
        self.was_ringing = ringing;
        started
    }

    pub fn ui(&mut self, ui: &mut Ui, shared: &mut Shared) {
        let p = shared.palette;
        let now = platform::local_time();

        // ── Es klingelt ──
        if let Some(what) = self.handle.read(|state| state.ringing.clone()) {
            theme::card(&p).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(td(what)).size(18.0).color(p.accent));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::primary_button(ui, &p, t("Stopp"), true).clicked() {
                            self.handle.update(|state| state.ringing = None);
                        }
                    });
                });
            });
        }

        self.time_card(ui, shared, &now);

        let (mut add_alarm, mut add_countdown, mut add_stopwatch) = (false, false, false);
        ui.columns(2, |columns| {
            add_alarm = self.alarms(&mut columns[0], &p, shared);
            add_countdown = self.countdowns(&mut columns[1], &p);
            add_stopwatch = self.stopwatches(&mut columns[1], &p);
        });
        if add_alarm {
            self.dialog = Some(Dialog::Alarm {
                hour: 7,
                minute: 0,
                label: String::new(),
                repeat: true,
            });
        }
        if add_countdown {
            self.dialog = Some(Dialog::Countdown {
                hours: 0,
                minutes: 5,
                seconds: 0,
                label: String::new(),
            });
        }
        if add_stopwatch {
            let id = self.next_stopwatch;
            self.next_stopwatch += 1;
            self.stopwatches.push(Stopwatch {
                id,
                label: String::new(),
                since: Some(Instant::now()),
                total: Duration::ZERO,
                laps: Vec::new(),
            });
        }
        self.dialogs(ui.ctx(), shared);

        // Sekundenanzeige und Stoppuhren laufen weiter.
        let running = self.stopwatches.iter().any(|watch| watch.since.is_some());
        let pace = if running { 60 } else { 250 };
        ui.ctx().request_repaint_after(Duration::from_millis(pace));
    }

    /// Uhrzeit mit den Vollbild-Knöpfen und den Beenden-Optionen.
    fn time_card(&mut self, ui: &mut Ui, shared: &mut Shared, now: &platform::LocalTime) {
        let p = shared.palette;
        theme::card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                let time = format!("{:02}:{:02}:{:02}", now.hour, now.minute, now.second);
                ui.label(RichText::new(time).size(if p.compact { 64.0 } else { 96.0 }));
                ui.label(theme::secondary(&p, format_date(now)).size(19.0));
            });
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                for (mode, label, hint) in [
                    (
                        Mode::Afk,
                        "AFK – Bildschirme schwarz",
                        "Die gewählten Bildschirme im Vollbild schwarz. Der Rechner bleibt wach.",
                    ),
                    (
                        Mode::Clock,
                        "Große Uhr",
                        "Wie AFK, aber mit Uhr auf jedem gewählten Bildschirm.",
                    ),
                    (
                        Mode::Spotify,
                        "Spotify-Vollbild",
                        "Zeigt die aktuelle Wiedergabe im Vollbild. Der Rechner bleibt wach.",
                    ),
                ] {
                    if ui.button(t(label)).on_hover_text(t(hint)).clicked() {
                        shared.blackout_request = Some(mode);
                    }
                }
                ui.label(theme::secondary(&p, t("Bildschirme:")).size(12.5));
                screen_picker(ui, shared);
            });
            ui.add_space(2.0);
            self.exit_options(ui, shared);
        });
    }

    /// Beenden-Tasten für die Vollbildmodi.
    fn exit_options(&mut self, ui: &mut Ui, shared: &mut Shared) {
        let p = shared.palette;
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            ui.label(theme::secondary(&p, t("Beenden mit:")).size(12.5));
            changed |= ui
                .checkbox(&mut shared.settings.afk_exit_f11, "F11")
                .changed();
            changed |= ui
                .checkbox(&mut shared.settings.afk_exit_esc, "Esc")
                .changed();
            changed |= ui
                .checkbox(&mut shared.settings.blackout_any_key, t("jede Taste"))
                .changed();

            // Eigene Taste: auf Knopfdruck die nächste Taste übernehmen.
            let current = egui::Key::from_name(&shared.settings.afk_custom_key);
            let label = if self.capturing_key {
                t("Taste drücken …").to_owned()
            } else {
                match current {
                    Some(key) => td(format!("Eigene Taste: {}", key.name())),
                    None => t("Eigene Taste festlegen").to_owned(),
                }
            };
            if ui.button(label).clicked() {
                self.capturing_key = !self.capturing_key;
            }
            if current.is_some()
                && ui
                    .small_button("×")
                    .on_hover_text(t("Eigene Taste entfernen"))
                    .clicked()
            {
                shared.settings.afk_custom_key.clear();
                changed = true;
            }
        });
        if self.capturing_key {
            let pressed = ui.ctx().input(|input| {
                input.events.iter().find_map(|event| match event {
                    egui::Event::Key {
                        key, pressed: true, ..
                    } => Some(*key),
                    _ => None,
                })
            });
            if let Some(key) = pressed {
                self.capturing_key = false;
                if key != egui::Key::Escape {
                    if blackout::virtual_key(key).is_some() {
                        shared.settings.afk_custom_key = key.name().to_owned();
                        changed = true;
                    } else {
                        shared.notify(t("Diese Taste wird nicht unterstützt."));
                    }
                }
            }
        }
        ui.label(
            theme::secondary(
                &p,
                t("Als Notausgang beendet Esc, 20 Sekunden lang gehalten, die Vollbildmodi immer."),
            )
            .size(12.0),
        );
        if changed {
            shared.save_settings();
        }
    }

    fn alarms(&mut self, ui: &mut Ui, p: &Palette, shared: &mut Shared) -> bool {
        let mut changed = false;
        let add = section(ui, p, "Wecker", Some("+ Wecker"), |ui| {
            let mut alarms: Vec<Alarm> = self.handle.read(|state| state.alarms.clone());
            let mut remove: Option<u64> = None;
            if alarms.is_empty() {
                ui.label(theme::secondary(p, t("Noch kein Wecker eingerichtet.")).size(14.5));
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
                    ui.label(RichText::new(time).size(34.0).color(color));
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
                    ui.label(theme::secondary(p, details).size(14.5));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .small_button("×")
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
            if changed {
                self.handle.update(|state| state.alarms = alarms);
            }
        });
        if changed {
            self.persist(shared);
        }
        add
    }

    /// Liste der Countdowns. Liefert `true`, wenn „+“ gedrückt wurde.
    fn countdowns(&mut self, ui: &mut Ui, p: &Palette) -> bool {
        let mut removed: Option<u64> = None;
        let add = section(ui, p, "Countdowns", Some("+ Countdown"), |ui| {
            let list: Vec<Countdown> = self.handle.read(|state| state.countdowns.clone());
            if list.is_empty() {
                ui.label(theme::secondary(p, t("Noch kein Countdown eingerichtet.")).size(14.5));
            }
            let now = clock::unix_now();
            for countdown in &list {
                ui.horizontal(|ui| {
                    let (shown, running) = match countdown.end {
                        Some(end) => (end.saturating_sub(now) as u32, true),
                        None => (countdown.secs, false),
                    };
                    let color = if running { p.accent } else { p.text };
                    ui.label(
                        RichText::new(format_span(Duration::from_secs(shown as u64), false))
                            .size(34.0)
                            .color(color),
                    );
                    if !countdown.label.trim().is_empty() {
                        ui.label(theme::secondary(p, countdown.label.trim()).size(14.5));
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .small_button("×")
                            .on_hover_text(t("Countdown löschen"))
                            .clicked()
                        {
                            removed = Some(countdown.id);
                        }
                        let label = if running { t("Stopp") } else { t("Start") };
                        if ui.button(label).clicked() {
                            let id = countdown.id;
                            self.handle.update(|state| {
                                if let Some(entry) =
                                    state.countdowns.iter_mut().find(|entry| entry.id == id)
                                {
                                    entry.end = if running {
                                        None
                                    } else {
                                        Some(clock::unix_now() + entry.secs as u64)
                                    };
                                }
                            });
                        }
                    });
                });
            }
        });
        if let Some(id) = removed {
            self.handle
                .update(|state| state.countdowns.retain(|entry| entry.id != id));
            // Gespeichert wird beim nächsten Hinzufügen bzw. über `persist` im Hintergrund.
            self.countdowns_dirty();
        }
        add
    }

    /// Merkt vor, dass die Countdown-Liste gespeichert werden muss.
    fn countdowns_dirty(&self) {
        self.handle.update(|state| state.dirty = true);
    }

    /// Liste der Stoppuhren. Liefert `true`, wenn „+“ gedrückt wurde.
    fn stopwatches(&mut self, ui: &mut Ui, p: &Palette) -> bool {
        let mut removed: Option<u64> = None;
        let add = section(ui, p, "Stoppuhren", Some("+ Stoppuhr"), |ui| {
            if self.stopwatches.is_empty() {
                ui.label(theme::secondary(p, t("Noch keine Stoppuhr.")).size(14.5));
            }
            let count = self.stopwatches.len();
            for (index, watch) in self.stopwatches.iter_mut().enumerate() {
                let running = watch.since.is_some();
                let elapsed = watch.elapsed();
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format_span(elapsed, true)).size(34.0));
                    ui.add(
                        egui::TextEdit::singleline(&mut watch.label)
                            .hint_text(t("Name"))
                            .desired_width(90.0),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .small_button("×")
                            .on_hover_text(t("Stoppuhr löschen"))
                            .clicked()
                        {
                            removed = Some(watch.id);
                        }
                        let can_reset = !running && elapsed > Duration::ZERO;
                        if ui
                            .add_enabled(can_reset, egui::Button::new(t("Zurücksetzen")))
                            .clicked()
                        {
                            watch.total = Duration::ZERO;
                            watch.laps.clear();
                        }
                        if ui
                            .add_enabled(running, egui::Button::new(t("Runde")))
                            .clicked()
                        {
                            watch.laps.push(elapsed);
                        }
                        let label = if running { t("Anhalten") } else { t("Starten") };
                        if theme::primary_button(ui, p, label, true).clicked() {
                            match watch.since.take() {
                                Some(since) => watch.total += since.elapsed(),
                                None => watch.since = Some(Instant::now()),
                            }
                        }
                    });
                });
                // Die letzten Runden.
                let mut previous = Duration::ZERO;
                let skip = watch.laps.len().saturating_sub(4);
                for (number, lap) in watch.laps.iter().enumerate() {
                    let split = lap.saturating_sub(previous);
                    previous = *lap;
                    if number < skip {
                        continue;
                    }
                    ui.label(
                        theme::secondary(
                            p,
                            format!(
                                "{}. {}  (+{})",
                                number + 1,
                                format_span(*lap, true),
                                format_span(split, true)
                            ),
                        )
                        .size(14.5),
                    );
                }
                if index + 1 < count {
                    ui.separator();
                }
            }
        });
        if let Some(id) = removed {
            self.stopwatches.retain(|watch| watch.id != id);
        }
        add
    }

    /// Eingabefenster für neue Wecker und Countdowns.
    fn dialogs(&mut self, ctx: &egui::Context, shared: &mut Shared) {
        let p = shared.palette;
        let Some(dialog) = self.dialog.as_mut() else {
            return;
        };
        let title = match dialog {
            Dialog::Alarm { .. } => t("Neuer Wecker"),
            Dialog::Countdown { .. } => t("Neuer Countdown"),
        };
        // `Some(true)` = hinzufügen und starten, `Some(false)` = nur hinzufügen.
        let mut confirmed: Option<bool> = None;
        let mut cancel = false;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| match dialog {
                Dialog::Alarm {
                    hour,
                    minute,
                    label,
                    repeat,
                } => {
                    ui.horizontal(|ui| {
                        ui.label(t("Uhrzeit"));
                        ui.add(egui::DragValue::new(hour).range(0..=23).suffix(" h"));
                        ui.add(egui::DragValue::new(minute).range(0..=59).suffix(" min"));
                    });
                    ui.add(
                        egui::TextEdit::singleline(label)
                            .hint_text(t("Bezeichnung"))
                            .desired_width(240.0),
                    );
                    ui.checkbox(repeat, t("Täglich wiederholen"));
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        if theme::primary_button(ui, &p, t("Hinzufügen"), true).clicked() {
                            confirmed = Some(false);
                        }
                        if ui.button(t("Abbrechen")).clicked() {
                            cancel = true;
                        }
                    });
                }
                Dialog::Countdown {
                    hours,
                    minutes,
                    seconds,
                    label,
                } => {
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(hours).range(0..=99).suffix(" h"));
                        ui.add(egui::DragValue::new(minutes).range(0..=59).suffix(" min"));
                        ui.add(egui::DragValue::new(seconds).range(0..=59).suffix(" s"));
                    });
                    ui.horizontal(|ui| {
                        for (name, preset) in
                            [("1 min", 1), ("5 min", 5), ("15 min", 15), ("30 min", 30)]
                        {
                            if ui.small_button(name).clicked() {
                                (*hours, *minutes, *seconds) = (0, preset, 0);
                            }
                        }
                    });
                    ui.add(
                        egui::TextEdit::singleline(label)
                            .hint_text(t("Bezeichnung"))
                            .desired_width(240.0),
                    );
                    ui.add_space(4.0);
                    let total = *hours * 3600 + *minutes * 60 + *seconds;
                    ui.horizontal(|ui| {
                        if theme::primary_button(ui, &p, t("Hinzufügen und starten"), total > 0)
                            .clicked()
                        {
                            confirmed = Some(true);
                        }
                        if ui
                            .add_enabled(total > 0, egui::Button::new(t("Hinzufügen")))
                            .clicked()
                        {
                            confirmed = Some(false);
                        }
                        if ui.button(t("Abbrechen")).clicked() {
                            cancel = true;
                        }
                    });
                }
            });
        if ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
            cancel = true;
        }
        if cancel {
            self.dialog = None;
            return;
        }
        let Some(start) = confirmed else {
            return;
        };
        match self.dialog.take() {
            Some(Dialog::Alarm {
                hour,
                minute,
                label,
                repeat,
            }) => {
                self.handle.update(|state| {
                    let id = state.alarms.iter().map(|alarm| alarm.id).max().unwrap_or(0) + 1;
                    state.alarms.push(Alarm {
                        id,
                        hour,
                        minute,
                        label: label.trim().to_owned(),
                        enabled: true,
                        repeat,
                    });
                    state.alarms.sort_by_key(|alarm| (alarm.hour, alarm.minute));
                });
            }
            Some(Dialog::Countdown {
                hours,
                minutes,
                seconds,
                label,
            }) => {
                let secs = hours * 3600 + minutes * 60 + seconds;
                self.handle.update(|state| {
                    let id = state
                        .countdowns
                        .iter()
                        .map(|entry| entry.id)
                        .max()
                        .unwrap_or(0)
                        + 1;
                    state.countdowns.push(Countdown {
                        id,
                        label: label.trim().to_owned(),
                        secs,
                        end: start.then(|| clock::unix_now() + secs as u64),
                    });
                });
            }
            None => {}
        }
        self.persist(shared);
    }
}
