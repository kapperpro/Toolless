use std::collections::BTreeSet;

use eframe::egui::{self, Align, Frame, Label, Layout, Margin, RichText, Stroke, TextEdit, Ui};

use crate::{
    app::Shared,
    services::clipboard::{self, ClipItem},
    theme,
};

/// Wie oft die System-Zwischenablage neu eingelesen wird (Sekunden).
const REFRESH_SECONDS: f64 = 1.0;

enum Action {
    Copy(ClipItem),
    Edit(ClipItem),
    Delete(i64),
    SaveEdit,
    CancelEdit,
}

#[derive(Default)]
pub struct ClipboardView {
    loaded: bool,
    items: Vec<ClipItem>,
    current: String,
    last_read: f64,
    title: String,
    category: String,
    /// `None` = alle Kategorien.
    filter: Option<String>,
    search: String,
    editing: Option<ClipItem>,
}

impl ClipboardView {
    fn refresh_current(&mut self, ui: &Ui, shared: &mut Shared) {
        if shared.now - self.last_read >= REFRESH_SECONDS || self.last_read == 0.0 {
            self.last_read = shared.now;
            if let Ok(text) = clipboard::read_system() {
                self.current = text;
            }
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(REFRESH_SECONDS));
    }

    fn save_current(&mut self, shared: &mut Shared) {
        match clipboard::save(&shared.db, &self.title, &self.category, &self.current) {
            Ok(item) => {
                shared.notify(format!("„{}“ gespeichert", item.title));
                self.items.insert(0, item);
                self.title.clear();
            }
            Err(err) => shared.notify(err),
        }
    }

    fn apply(&mut self, action: Action, shared: &mut Shared) {
        match action {
            Action::Copy(item) => match clipboard::write_system(&item.content) {
                Ok(()) => {
                    self.current = item.content;
                    shared.notify(format!("„{}“ kopiert", item.title));
                }
                Err(err) => shared.notify(err),
            },
            Action::Edit(item) => self.editing = Some(item),
            Action::Delete(id) => match clipboard::delete(&shared.db, id) {
                Ok(()) => self.items.retain(|item| item.id != id),
                Err(err) => shared.notify(err),
            },
            Action::SaveEdit => {
                if let Some(draft) = self.editing.take() {
                    match clipboard::update(&shared.db, &draft) {
                        Ok(updated) => {
                            if let Some(slot) =
                                self.items.iter_mut().find(|item| item.id == updated.id)
                            {
                                *slot = updated;
                            }
                        }
                        Err(err) => {
                            self.editing = Some(draft);
                            shared.notify(err);
                        }
                    }
                }
            }
            Action::CancelEdit => self.editing = None,
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, shared: &mut Shared) {
        let p = shared.palette;
        if !self.loaded {
            self.loaded = true;
            match clipboard::list(&shared.db) {
                Ok(items) => self.items = items,
                Err(err) => shared.notify(err),
            }
        }
        self.refresh_current(ui, shared);

        // ── Aktuelle Zwischenablage + Speichern ──
        theme::card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::card_title(ui, "Aktuelle Zwischenablage");
            Frame::none()
                .fill(p.field_bg)
                .stroke(Stroke::new(1.0, p.separator))
                .rounding(8.0)
                .inner_margin(Margin::symmetric(12.0, 9.0))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    if self.current.is_empty() {
                        ui.label(theme::secondary(
                            &p,
                            "Die Zwischenablage enthält gerade keinen Text.",
                        ));
                    } else {
                        let preview: String = self.current.chars().take(400).collect();
                        ui.label(RichText::new(preview).monospace());
                    }
                });

            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let can_save = !self.current.is_empty();
                    let clicked = theme::primary_button(ui, &p, "Speichern", can_save).clicked();
                    let category = ui.add(
                        TextEdit::singleline(&mut self.category)
                            .hint_text("Kategorie")
                            .desired_width(170.0),
                    );
                    let title = ui.add(
                        TextEdit::singleline(&mut self.title)
                            .hint_text("Titel (optional)")
                            .desired_width(f32::INFINITY),
                    );
                    let enter = (title.lost_focus() || category.lost_focus())
                        && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if can_save && (clicked || enter) {
                        self.save_current(shared);
                    }
                });
            });
        });

        // ── Filter: Kategorien + Suche ──
        let categories: BTreeSet<String> = self
            .items
            .iter()
            .filter(|item| !item.category.is_empty())
            .map(|item| item.category.clone())
            .collect();
        if self
            .filter
            .as_ref()
            .is_some_and(|f| !categories.contains(f))
        {
            self.filter = None;
        }
        ui.horizontal_wrapped(|ui| {
            if theme::chip(ui, &p, "Alle", self.filter.is_none()).clicked() {
                self.filter = None;
            }
            for name in &categories {
                if theme::chip(ui, &p, name, self.filter.as_ref() == Some(name)).clicked() {
                    self.filter = Some(name.clone());
                }
            }
            ui.add(
                TextEdit::singleline(&mut self.search)
                    .hint_text("Suchen")
                    .desired_width(180.0),
            );
        });

        // ── Gespeicherte Einträge ──
        let needle = self.search.trim().to_lowercase();
        let visible: Vec<ClipItem> = self
            .items
            .iter()
            .filter(|item| self.filter.as_ref().map_or(true, |f| &item.category == f))
            .filter(|item| {
                needle.is_empty()
                    || item.title.to_lowercase().contains(&needle)
                    || item.content.to_lowercase().contains(&needle)
                    || item.category.to_lowercase().contains(&needle)
            })
            .cloned()
            .collect();

        if visible.is_empty() {
            let text = if self.items.is_empty() {
                "Noch nichts gespeichert. Kopiere etwas und sichere es oben mit Titel und Kategorie."
            } else {
                "Keine Einträge für diesen Filter."
            };
            theme::empty_state(ui, &p, text);
            return;
        }

        let mut action = None;
        theme::card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let count = visible.len();
            for (index, item) in visible.into_iter().enumerate() {
                match self.editing.as_mut().filter(|draft| draft.id == item.id) {
                    Some(draft) => {
                        ui.horizontal(|ui| {
                            ui.add(
                                TextEdit::singleline(&mut draft.title)
                                    .hint_text("Titel")
                                    .desired_width(260.0),
                            );
                            ui.add(
                                TextEdit::singleline(&mut draft.category)
                                    .hint_text("Kategorie")
                                    .desired_width(170.0),
                            );
                        });
                        ui.add(
                            TextEdit::multiline(&mut draft.content)
                                .font(egui::TextStyle::Monospace)
                                .desired_rows(4)
                                .desired_width(f32::INFINITY),
                        );
                        ui.horizontal(|ui| {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if theme::primary_button(ui, &p, "Sichern", true).clicked() {
                                    action = Some(Action::SaveEdit);
                                }
                                if ui.button("Abbrechen").clicked() {
                                    action = Some(Action::CancelEdit);
                                }
                            });
                        });
                    }
                    None => {
                        ui.horizontal(|ui| {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui
                                    .button(RichText::new("Löschen").color(p.danger))
                                    .clicked()
                                {
                                    action = Some(Action::Delete(item.id));
                                }
                                if ui.button("Bearbeiten").clicked() {
                                    action = Some(Action::Edit(item.clone()));
                                }
                                if theme::primary_button(ui, &p, "Kopieren", true).clicked() {
                                    action = Some(Action::Copy(item.clone()));
                                }
                                // Restbreite: Titel, Kategorie und Inhaltsvorschau.
                                ui.with_layout(Layout::top_down(Align::Min), |ui| {
                                    ui.horizontal(|ui| {
                                        ui.add(
                                            Label::new(RichText::new(&item.title).strong())
                                                .truncate(),
                                        );
                                        if !item.category.is_empty() {
                                            ui.label(
                                                RichText::new(&item.category)
                                                    .size(12.0)
                                                    .color(p.accent),
                                            );
                                        }
                                    });
                                    ui.add(
                                        Label::new(
                                            theme::secondary(
                                                &p,
                                                theme::first_line(&item.content, 120),
                                            )
                                            .size(12.5),
                                        )
                                        .truncate(),
                                    );
                                });
                            });
                        });
                    }
                }
                if index + 1 < count {
                    ui.separator();
                }
            }
        });

        if let Some(action) = action {
            self.apply(action, shared);
        }
    }
}
