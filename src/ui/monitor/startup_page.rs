//! Seite „Autostart“ im Task-Manager: Programme, die mit Windows starten – ein- und
//! ausschaltbar wie in der Registerkarte „Autostart“ des Windows-Task-Managers.

use std::{
    sync::mpsc::{self, Receiver},
    thread,
};

use eframe::egui::{self, Align, Align2, FontId, Layout, Rect, RichText, Sense, TextEdit, Ui};

use crate::{
    app::Shared,
    i18n::{t, td},
    services::startup::{self, Entry, Source},
    theme,
};

const ROW_HEIGHT: f32 = 30.0;

/// Spalte, nach der sortiert wird.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum SortColumn {
    #[default]
    Name,
    Status,
    Source,
    Command,
}

#[derive(Default)]
pub struct StartupView {
    sort: SortColumn,
    /// Absteigend sortieren.
    reversed: bool,
    entries: Option<Vec<Entry>>,
    loading: Option<Receiver<Vec<Entry>>>,
    /// Markierter Eintrag (Herkunft und Name).
    selected: Option<(Source, String)>,
    search: String,
}

impl StartupView {
    pub(super) fn reload(&mut self, ctx: &egui::Context) {
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        thread::spawn(move || {
            let _ = sender.send(startup::list());
            ctx.request_repaint();
        });
        self.loading = Some(receiver);
    }

    /// Schaltet den markierten Eintrag um.
    fn toggle(&mut self, shared: &mut Shared, target: &(Source, String)) {
        let Some(entries) = self.entries.as_mut() else {
            return;
        };
        let Some(entry) = entries
            .iter_mut()
            .find(|entry| entry.source == target.0 && entry.key == target.1)
        else {
            return;
        };
        let enable = !entry.enabled;
        match startup::set_enabled(entry, enable) {
            Ok(()) => entry.enabled = enable,
            Err(error) => {
                let hint = if entry.source == Source::User || entry.source == Source::Folder {
                    ""
                } else {
                    " Einträge für alle Benutzer brauchen Administratorrechte."
                };
                shared.notify(td(format!(
                    "Autostart ließ sich nicht ändern: {error}.{hint}"
                )));
            }
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, shared: &mut Shared) {
        let p = shared.palette;
        if self.entries.is_none() && self.loading.is_none() {
            self.reload(ui.ctx());
        }
        let received = self.loading.as_ref().and_then(|rx| rx.try_recv().ok());
        if let Some(list) = received {
            self.entries = Some(list);
            self.loading = None;
        }

        let needle = self.search.trim().to_lowercase();
        let rows: Vec<Entry> = self
            .entries
            .iter()
            .flatten()
            .filter(|entry| {
                needle.is_empty()
                    || entry.name.to_lowercase().contains(&needle)
                    || entry.command.to_lowercase().contains(&needle)
            })
            .cloned()
            .collect();
        let mut rows = rows;
        match self.sort {
            SortColumn::Name => rows.sort_by_key(|entry| entry.name.to_lowercase()),
            SortColumn::Status => {
                rows.sort_by_key(|entry| (!entry.enabled, entry.name.to_lowercase()))
            }
            SortColumn::Source => {
                rows.sort_by_key(|entry| (entry.source.label(), entry.name.to_lowercase()))
            }
            SortColumn::Command => rows.sort_by_key(|entry| entry.command.to_lowercase()),
        }
        if self.reversed {
            rows.reverse();
        }
        let selected_entry = self.selected.as_ref().and_then(|(source, key)| {
            rows.iter()
                .find(|entry| entry.source == *source && entry.key == *key)
                .cloned()
        });
        if selected_entry.is_none() {
            self.selected = None;
        }

        let mut toggle: Option<(Source, String)> = None;
        let mut clear = false;
        let mut refresh = false;
        ui.horizontal(|ui| {
            if ui
                .button(t("Aktualisieren"))
                .on_hover_text(t("Liest die Autostart-Einträge neu ein"))
                .clicked()
            {
                refresh = true;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = match &selected_entry {
                    Some(entry) if !entry.enabled => t("Aktivieren"),
                    _ => t("Deaktivieren"),
                };
                if ui
                    .add_enabled(selected_entry.is_some(), egui::Button::new(label))
                    .on_hover_text(t(
                        "Schaltet den markierten Eintrag ein oder aus (Doppelklick geht auch)",
                    ))
                    .clicked()
                {
                    toggle = self.selected.clone();
                }
                if ui
                    .add_enabled(
                        selected_entry.is_some(),
                        egui::Button::new(t("Auswahl aufheben")),
                    )
                    .on_hover_text(t("Hebt die Markierung in der Liste auf (Esc)"))
                    .clicked()
                {
                    clear = true;
                }
                ui.label(theme::secondary(&p, td(format!("{} Einträge", rows.len()))).size(12.5));
                ui.add(
                    TextEdit::singleline(&mut self.search)
                        .hint_text(t("Autostart durchsuchen"))
                        .desired_width(f32::INFINITY),
                );
            });
        });
        ui.add_space(4.0);

        if self.entries.is_none() {
            ui.label(theme::secondary(
                &p,
                t("Autostart-Einträge werden gelesen …"),
            ));
        } else if rows.is_empty() {
            ui.label(theme::secondary(
                &p,
                t("Keine Autostart-Einträge gefunden."),
            ));
        }

        if self.selected.is_some() && ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            clear = true;
        }

        let mut widths =
            super::column_widths(&shared.settings.startup_columns, [300.0, 110.0, 170.0]);
        let mut widths_changed = false;
        let mut widths_saved = false;
        theme::card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 1.0;

            let (header, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), Sense::hover());
            let columns = column_edges(header, widths);
            let columns_all = [
                (SortColumn::Name, "Name"),
                (SortColumn::Status, "Status"),
                (SortColumn::Source, "Herkunft"),
                (SortColumn::Command, "Befehl"),
            ];
            for (index, (column, label)) in columns_all.into_iter().enumerate() {
                let right = columns.get(index + 1).copied().unwrap_or(header.right());
                let click = Rect::from_min_max(
                    egui::pos2(columns[index] - 6.0, header.top()),
                    egui::pos2(right - 8.0, header.bottom()),
                );
                let response = ui
                    .interact(click, ui.id().with(("startup-sort", index)), Sense::click())
                    .on_hover_cursor(egui::CursorIcon::PointingHand);
                let active = self.sort == column;
                let text_rect = ui.painter().text(
                    egui::pos2(columns[index], header.center().y),
                    Align2::LEFT_CENTER,
                    t(label),
                    FontId::proportional(12.5),
                    if active { p.text } else { p.text_secondary },
                );
                if active {
                    super::sort_arrow(
                        ui,
                        egui::pos2(text_rect.right() + 11.0, text_rect.center().y),
                        p.text,
                        self.reversed,
                    );
                }
                if response.clicked() {
                    if active {
                        self.reversed = !self.reversed;
                    } else {
                        self.sort = column;
                        self.reversed = false;
                    }
                }
                // Spaltengrenze rechts der Spalte (nicht nach dem Befehl).
                if index < 3 {
                    let (delta, stopped) = super::resize_handle(
                        ui,
                        ui.id().with(("startup-resize", index)),
                        right - 10.0,
                        header.top(),
                        header.bottom(),
                        &p,
                    );
                    if delta != 0.0 {
                        widths[index] = (widths[index] + delta).clamp(60.0, 700.0);
                        widths_changed = true;
                    }
                    widths_saved |= stopped;
                }
            }
            ui.separator();

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for entry in &rows {
                        let (rect, response) = ui.allocate_exact_size(
                            egui::vec2(ui.available_width(), ROW_HEIGHT),
                            Sense::click(),
                        );
                        let key = (entry.source, entry.key.clone());
                        let is_selected = self.selected.as_ref() == Some(&key);
                        if is_selected {
                            ui.painter().rect_filled(rect, p.radius, p.selection);
                        } else if response.hovered() {
                            ui.painter().rect_filled(
                                rect,
                                p.radius,
                                p.selection.gamma_multiply(0.5),
                            );
                        }
                        let columns = column_edges(rect, widths);
                        let center = rect.center().y;
                        let cell = |from: f32, to: f32| {
                            ui.painter().with_clip_rect(Rect::from_min_max(
                                egui::pos2(from, rect.top()),
                                egui::pos2(to - 8.0, rect.bottom()),
                            ))
                        };
                        let name_color = if entry.enabled {
                            p.text
                        } else {
                            p.text_secondary
                        };
                        cell(columns[0], columns[1]).text(
                            egui::pos2(columns[0], center),
                            Align2::LEFT_CENTER,
                            &entry.name,
                            FontId::proportional(14.0),
                            name_color,
                        );
                        let (status, status_color) = if entry.enabled {
                            (t("Aktiviert"), p.success)
                        } else {
                            (t("Deaktiviert"), p.text_secondary)
                        };
                        cell(columns[1], columns[2]).text(
                            egui::pos2(columns[1], center),
                            Align2::LEFT_CENTER,
                            status,
                            FontId::proportional(13.0),
                            status_color,
                        );
                        cell(columns[2], columns[3]).text(
                            egui::pos2(columns[2], center),
                            Align2::LEFT_CENTER,
                            t(entry.source.label()),
                            FontId::proportional(13.0),
                            p.text_secondary,
                        );
                        cell(columns[3], rect.right()).text(
                            egui::pos2(columns[3], center),
                            Align2::LEFT_CENTER,
                            &entry.command,
                            FontId::proportional(12.0),
                            p.text_secondary,
                        );
                        let response = response.on_hover_text(entry.command.as_str());
                        if response.clicked() {
                            self.selected = Some(key.clone());
                        }
                        if response.double_clicked() {
                            toggle = Some(key);
                        }
                    }
                });
        });

        ui.add_space(4.0);
        ui.label(
            RichText::new(t(
                "Deaktivierte Programme bleiben installiert und starten nur nicht mehr mit Windows. Einträge für „Alle Benutzer“ brauchen Administratorrechte.",
            ))
            .size(12.0)
            .color(p.text_secondary),
        );

        if widths_changed {
            shared.settings.startup_columns = widths.to_vec();
        }
        if widths_saved {
            shared.save_settings();
        }
        if clear {
            self.selected = None;
        }
        if let Some(target) = toggle {
            self.toggle(shared, &target);
        }
        if refresh {
            self.entries = None;
        }
    }
}

/// Linke Kanten der Spalten: Name, Status, Herkunft, Befehl.
fn column_edges(rect: Rect, widths: [f32; 3]) -> [f32; 4] {
    let first = rect.left() + 10.0;
    let second = first + widths[0];
    let third = second + widths[1];
    [first, second, third, third + widths[2]]
}
