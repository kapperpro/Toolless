//! Game Timer: Spielzeit je Spiel, automatisch erkannt oder über eigene Programme.

use eframe::egui::{self, Align, Align2, FontId, Layout, RichText, Sense, TextEdit, Ui};

use crate::{
    app::Shared,
    i18n::{t, td},
    platform,
    services::{
        games::{self, Game, Library},
        monitor::Command,
    },
    theme::{self, Palette},
};

const DAY: u64 = 86_400;

enum Action {
    AddFile,
    AddRunning(String, String),
    RemoveCustom(usize),
    Ignore(String),
    Unignore(usize),
    AskRemove(String),
    Remove(String),
    Export,
}

#[derive(Default)]
pub struct GamesView {
    search: String,
    /// Wartet auf Bestätigung: Zeiten dieses Spiels löschen.
    confirm_remove: Option<String>,
}

/// „3 h 05 min“, „12 min“ oder „40 s“.
fn format_duration(secs: u64) -> String {
    match secs {
        0 => "–".to_owned(),
        1..=59 => format!("{secs} s"),
        60..=3599 => format!("{} min", secs / 60),
        _ => format!("{} h {:02} min", secs / 3600, secs % 3600 / 60),
    }
}

fn format_date_time(time: u64) -> String {
    let (year, month, day, hour, minute) = platform::local_parts(time);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

fn format_day(time: u64) -> String {
    let (_, month, day, hour, minute) = platform::local_parts(time);
    format!("{day:02}.{month:02}. {hour:02}:{minute:02}")
}

/// Kennzahl mit Beschriftung darüber.
fn stat(ui: &mut Ui, p: &Palette, label: &'static str, value: String) {
    ui.vertical_centered(|ui| {
        ui.label(theme::secondary(p, t(label)).size(12.5));
        ui.label(RichText::new(value).size(22.0).color(p.accent));
    });
}

impl GamesView {
    fn apply(&mut self, action: Action, library: &Library, shared: &mut Shared) {
        let mut config_changed = true;
        match action {
            Action::AddFile => {
                let picked = rfd::FileDialog::new()
                    .set_title(t("Programm für den Game Timer auswählen"))
                    .pick_files()
                    .unwrap_or_default();
                for file in picked {
                    let path = file.to_string_lossy().into_owned();
                    let name = games::suggested_name(&path);
                    let custom = &mut shared.settings.games_custom;
                    if !custom.iter().any(|(_, existing)| existing == &path) {
                        custom.push((name, path));
                    }
                }
            }
            Action::AddRunning(name, exe) => {
                let custom = &mut shared.settings.games_custom;
                if !custom.iter().any(|(_, existing)| existing == &exe) {
                    custom.push((name, exe));
                }
            }
            Action::RemoveCustom(index) => {
                if index < shared.settings.games_custom.len() {
                    shared.settings.games_custom.remove(index);
                }
            }
            Action::Ignore(name) => {
                if !shared.settings.games_ignored.contains(&name) {
                    shared.settings.games_ignored.push(name);
                }
            }
            Action::Unignore(index) => {
                if index < shared.settings.games_ignored.len() {
                    shared.settings.games_ignored.remove(index);
                }
            }
            Action::AskRemove(name) => {
                self.confirm_remove = Some(name);
                config_changed = false;
            }
            Action::Remove(name) => {
                shared.monitor.send(Command::RemoveGame(name));
                self.confirm_remove = None;
                config_changed = false;
            }
            Action::Export => {
                config_changed = false;
                let target = rfd::FileDialog::new()
                    .set_title(t("Spielzeiten exportieren"))
                    .set_file_name("toolless-spielzeiten.csv")
                    .add_filter("CSV", &["csv"])
                    .save_file();
                if let Some(target) = target {
                    // Mit BOM, damit Excel die Umlaute richtig liest.
                    let csv = format!("\u{feff}{}", games::export_csv(library, format_date_time));
                    match std::fs::write(&target, csv) {
                        Ok(()) => {
                            shared.notify(td(format!("Exportiert nach {}", target.display())))
                        }
                        Err(err) => shared.notify(td(format!("Export fehlgeschlagen: {err}"))),
                    }
                }
            }
        }
        if config_changed {
            shared.save_settings();
            shared
                .monitor
                .send(Command::Games(shared.settings.game_config()));
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, shared: &mut Shared) {
        let p = shared.palette;
        let (library, running, apps): (Library, Vec<String>, Vec<(String, String)>) =
            shared.monitor.read(|state| {
                (
                    state.games.clone(),
                    state.running_games.clone(),
                    state
                        .processes
                        .iter()
                        .filter(|row| row.is_app)
                        .filter_map(|row| Some((row.name.clone(), row.exe.clone()?)))
                        .collect(),
                )
            });
        let mut action: Option<Action> = None;

        let now = games_now();
        let today = now.saturating_sub(platform::local_time().seconds_of_day() as u64);
        let week = today.saturating_sub(6 * DAY);
        let sum = |since: u64| -> u64 {
            library
                .games
                .iter()
                .map(|game| game.secs_between(since, now))
                .sum()
        };

        // ── Übersicht ──
        theme::card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.columns(3, |columns| {
                stat(&mut columns[0], &p, "Heute", format_duration(sum(today)));
                stat(
                    &mut columns[1],
                    &p,
                    "Letzte 7 Tage",
                    format_duration(sum(week)),
                );
                let total = library.games.iter().map(|game| game.total_secs).sum();
                stat(&mut columns[2], &p, "Gesamt", format_duration(total));
            });
            ui.add_space(4.0);
            if running.is_empty() {
                ui.vertical_centered(|ui| {
                    ui.label(
                        theme::secondary(&p, t("Gerade läuft kein erkanntes Spiel.")).size(12.5),
                    );
                });
            }
            for name in &running {
                let session = library
                    .games
                    .iter()
                    .find(|game| &game.name == name)
                    .and_then(|game| game.sessions.last())
                    .map_or(0, |session| session.secs);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(td(format!(
                            "Läuft gerade: {name} · Sitzung {}",
                            format_duration(session)
                        )))
                        .color(p.success),
                    );
                });
            }

            // Balken der letzten sieben Tage.
            let days: Vec<(u64, u64)> = (0..7)
                .map(|index| {
                    let start = week + index * DAY;
                    let secs = library
                        .games
                        .iter()
                        .map(|game| game.secs_between(start, (start + DAY).min(now)))
                        .sum();
                    (start, secs)
                })
                .collect();
            let peak = days.iter().map(|(_, secs)| *secs).max().unwrap_or(0).max(1);
            ui.add_space(6.0);
            let (rect, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 96.0), Sense::hover());
            let slot = rect.width() / 7.0;
            for (index, (start, secs)) in days.iter().enumerate() {
                let x = rect.left() + slot * (index as f32 + 0.5);
                let bar_height = (rect.height() - 34.0) * (*secs as f32 / peak as f32);
                let bar = egui::Rect::from_min_max(
                    egui::pos2(x - slot * 0.28, rect.bottom() - 16.0 - bar_height.max(1.0)),
                    egui::pos2(x + slot * 0.28, rect.bottom() - 16.0),
                );
                ui.painter()
                    .rect_filled(bar, 3.0, p.accent.gamma_multiply(0.8));
                let (_, month, day, _, _) = platform::local_parts(*start + DAY / 2);
                ui.painter().text(
                    egui::pos2(x, rect.bottom()),
                    Align2::CENTER_BOTTOM,
                    format!("{day:02}.{month:02}."),
                    FontId::proportional(11.0),
                    p.text_secondary,
                );
                if *secs > 0 {
                    ui.painter().text(
                        egui::pos2(x, bar.top() - 2.0),
                        Align2::CENTER_BOTTOM,
                        format_duration(*secs),
                        FontId::proportional(11.0),
                        p.text,
                    );
                }
            }
        });

        // ── Werkzeugleiste ──
        ui.horizontal(|ui| {
            ui.add(
                TextEdit::singleline(&mut self.search)
                    .hint_text(t("Spiel suchen"))
                    .desired_width(220.0),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add_enabled(
                        !library.games.is_empty(),
                        egui::Button::new(t("Exportieren (.csv)")),
                    )
                    .on_hover_text(t("Eine Zeile je Sitzung – lässt sich in Excel öffnen"))
                    .clicked()
                {
                    action = Some(Action::Export);
                }
                if ui
                    .button(t("Eigene App hinzufügen …"))
                    .on_hover_text(t("Zählt die Laufzeit eines beliebigen Programms"))
                    .clicked()
                {
                    action = Some(Action::AddFile);
                }
                ui.menu_button(t("Laufendes Programm hinzufügen"), |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(320.0)
                        .show(ui, |ui| {
                            for (name, exe) in &apps {
                                let title = games::suggested_name(name);
                                if ui.button(&title).on_hover_text(exe).clicked() {
                                    action = Some(Action::AddRunning(title, exe.clone()));
                                    ui.close_menu();
                                }
                            }
                        });
                });
            });
        });

        // Sicherheitsabfrage vor dem Löschen.
        if let Some(name) = self.confirm_remove.clone() {
            theme::card(&p).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(td(format!("Alle Zeiten von „{name}“ löschen?")));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button(t("Abbrechen")).clicked() {
                            self.confirm_remove = None;
                        }
                        if ui
                            .button(RichText::new(t("Löschen")).color(p.danger))
                            .clicked()
                        {
                            action = Some(Action::Remove(name.clone()));
                        }
                    });
                });
            });
        }

        // ── Spiele ──
        let needle = self.search.trim().to_lowercase();
        let mut shown: Vec<&Game> = library
            .games
            .iter()
            .filter(|game| needle.is_empty() || game.name.to_lowercase().contains(&needle))
            .collect();
        shown.sort_by_key(|game| std::cmp::Reverse(game.last_played()));

        theme::card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if shown.is_empty() {
                ui.label(theme::secondary(
                    &p,
                    t("Noch keine Spielzeit erfasst. Starte ein Spiel – Toolless erkennt Spiele aus Steam, Epic, \
                       Riot, GOG, Ubisoft, EA und Xbox sowie Minecraft von selbst. Alles andere fügst du oben hinzu."),
                ));
                return;
            }
            egui::Grid::new("games-table")
                .num_columns(6)
                .striped(true)
                .spacing(egui::vec2(26.0, 7.0))
                .show(ui, |ui| {
                    for title in ["Spiel", "Heute", "7 Tage", "Gesamt", "Sitzungen", "Zuletzt gespielt"] {
                        ui.label(theme::secondary(&p, t(title)).size(12.5));
                    }
                    ui.end_row();
                    for game in shown {
                        let is_running = running.contains(&game.name);
                        let name = ui
                            .horizontal(|ui| {
                                let response = ui.add(
                                    egui::Label::new(RichText::new(&game.name).strong())
                                        .sense(Sense::click()),
                                );
                                if is_running {
                                    theme::badge(ui, &p, t("läuft"));
                                }
                                response
                            })
                            .inner;
                        name.context_menu(|ui| {
                            if ui
                                .button(t("Nicht mehr als Spiel zählen"))
                                .on_hover_text(t("Bereits erfasste Zeiten bleiben erhalten"))
                                .clicked()
                            {
                                action = Some(Action::Ignore(game.name.clone()));
                                ui.close_menu();
                            }
                            if ui
                                .button(RichText::new(t("Zeiten löschen …")).color(p.danger))
                                .clicked()
                            {
                                action = Some(Action::AskRemove(game.name.clone()));
                                ui.close_menu();
                            }
                        });
                        ui.label(format_duration(game.secs_between(today, now)));
                        ui.label(format_duration(game.secs_between(week, now)));
                        ui.label(RichText::new(format_duration(game.total_secs)).strong());
                        ui.label(game.sessions.len().to_string());
                        ui.label(if is_running {
                            t("jetzt").to_owned()
                        } else {
                            format_day(game.last_played())
                        });
                        ui.end_row();
                    }
                });
            ui.add_space(2.0);
            ui.label(theme::secondary(&p, t("Rechtsklick auf ein Spiel: ausschließen oder Zeiten löschen.")).size(12.0));
        });

        // ── Eigene Programme und Ausschlüsse ──
        if !shared.settings.games_custom.is_empty() || !shared.settings.games_ignored.is_empty() {
            theme::card(&p).show(ui, |ui| {
                ui.set_width(ui.available_width());
                if !shared.settings.games_custom.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(theme::secondary(&p, t("Eigene Programme:")).size(12.5));
                        for (index, (name, path)) in shared.settings.games_custom.iter().enumerate()
                        {
                            if ui
                                .small_button(format!("{name}  ✕"))
                                .on_hover_text(path)
                                .clicked()
                            {
                                action = Some(Action::RemoveCustom(index));
                            }
                        }
                    });
                }
                if !shared.settings.games_ignored.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(theme::secondary(&p, t("Ausgeschlossen:")).size(12.5));
                        for (index, name) in shared.settings.games_ignored.iter().enumerate() {
                            if ui
                                .small_button(format!("{name}  ✕"))
                                .on_hover_text(t("Wieder als Spiel zählen"))
                                .clicked()
                            {
                                action = Some(Action::Unignore(index));
                            }
                        }
                    });
                }
            });
        }

        if let Some(action) = action {
            self.apply(action, &library, shared);
        }
        // Laufende Sitzungen zählen sichtbar weiter.
        if !running.is_empty() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs(2));
        }
    }
}

fn games_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}
