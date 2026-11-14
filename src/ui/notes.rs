use eframe::egui::{
    self, Align, CentralPanel, Frame, Layout, Margin, RichText, ScrollArea, SidePanel, TextEdit, Ui,
};

use crate::{
    app::Shared,
    services::notes::{now_ms, Note},
    theme,
};

#[derive(Default)]
pub struct NotesView {
    loaded: bool,
    notes: Vec<Note>,
    active: Option<String>,
}

impl NotesView {
    fn add(&mut self, shared: &mut Shared) {
        match shared.notes.create() {
            Ok(note) => {
                self.active = Some(note.id.clone());
                self.notes.insert(0, note);
            }
            Err(err) => shared.notify(err),
        }
    }

    fn delete_active(&mut self, shared: &mut Shared) {
        let Some(id) = self.active.take() else { return };
        match shared.notes.delete(&id) {
            Ok(()) => {
                self.notes.retain(|note| note.id != id);
                self.active = self.notes.first().map(|note| note.id.clone());
            }
            Err(err) => {
                self.active = Some(id);
                shared.notify(err);
            }
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, shared: &mut Shared) {
        let p = shared.palette;
        if !self.loaded {
            self.loaded = true;
            match shared.notes.list() {
                Ok(notes) => {
                    self.active = notes.first().map(|note| note.id.clone());
                    self.notes = notes;
                }
                Err(err) => shared.notify(err),
            }
        }

        // ── Linke Spalte: Notizliste ──
        let mut add_clicked = false;
        SidePanel::left("notes_list")
            .exact_width(230.0)
            .resizable(false)
            .frame(Frame::none().inner_margin(Margin {
                left: 0.0,
                right: 12.0,
                top: 0.0,
                bottom: 0.0,
            }))
            .show_inside(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        theme::secondary(&p, format!("{} Notizen", self.notes.len())).size(12.0),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        add_clicked = ui.button("Neue Notiz").clicked();
                    });
                });
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for note in &self.notes {
                            let title = if note.title.trim().is_empty() {
                                "Neue Notiz"
                            } else {
                                note.title.as_str()
                            };
                            let selected = self.active.as_deref() == Some(note.id.as_str());
                            let row = theme::nav_item(ui, &p, title, selected)
                                .on_hover_text(theme::first_line(&note.body, 80));
                            if row.clicked() {
                                self.active = Some(note.id.clone());
                            }
                        }
                    });
            });
        if add_clicked {
            self.add(shared);
        }

        // ── Rechte Spalte: Editor ──
        let mut delete_clicked = false;
        let mut save_error = None;
        CentralPanel::default()
            .frame(Frame::none().inner_margin(Margin {
                left: 12.0,
                right: 0.0,
                top: 0.0,
                bottom: 0.0,
            }))
            .show_inside(ui, |ui| {
                let active_id = self.active.clone();
                let Some(note) = self
                    .notes
                    .iter_mut()
                    .find(|note| Some(&note.id) == active_id.as_ref())
                else {
                    theme::empty_state(ui, &p, "Keine Notiz ausgewählt. Lege links eine neue an.");
                    return;
                };

                let mut changed = false;
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        delete_clicked = ui
                            .button(RichText::new("Löschen").color(p.danger))
                            .clicked();
                        changed |= ui
                            .add(
                                TextEdit::singleline(&mut note.title)
                                    .hint_text("Titel")
                                    .font(egui::TextStyle::Heading)
                                    .frame(false)
                                    .desired_width(f32::INFINITY),
                            )
                            .changed();
                    });
                });
                ui.separator();
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        changed |= ui
                            .add(
                                TextEdit::multiline(&mut note.body)
                                    .hint_text("Schreib los …")
                                    .frame(false)
                                    .desired_rows(24)
                                    .desired_width(f32::INFINITY),
                            )
                            .changed();
                    });

                // Jede Änderung sofort auf die Platte schreiben – die Dateien sind winzig.
                if changed {
                    note.updated_at = now_ms();
                    if let Err(err) = shared.notes.save(note) {
                        save_error = Some(err);
                    }
                }
            });

        if let Some(err) = save_error {
            shared.notify(err);
        }
        if delete_clicked {
            self.delete_active(shared);
        }
    }
}
