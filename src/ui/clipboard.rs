use crate::i18n::t;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use eframe::egui::{
    self, Align, Label, Layout, Rect, RichText, Sense, TextEdit, TextureHandle, Ui,
};

use crate::{
    app::Shared,
    services::clipboard::{self, ClipItem},
    theme,
};

/// Wie oft die System-Zwischenablage neu eingelesen wird (Sekunden).
const REFRESH_SECONDS: f64 = 1.0;
const ROW_HEIGHT: f32 = 46.0;

enum Action {
    Copy(ClipItem),
    Edit(ClipItem),
    Delete(i64),
    SaveEdit,
    CancelEdit,
    /// Eintrag (ID) in eine Kategorie verschieben (leer = ohne Kategorie).
    Move(i64, String),
    /// Kategorie vor eine andere einsortieren (gezogene, Ziel).
    Reorder(String, String),
}

/// Nutzlast beim Ziehen: ein Eintrag oder eine ganze Kategorie.
enum ClipDrag {
    Item(i64),
    Category(String),
}

const SETTING_CATEGORY_ORDER: &str = "clip_category_order";

#[derive(Default)]
pub struct ClipboardView {
    /// Vorschaubilder der Bild-Einträge je Dateiname; `broken` merkt sich nicht ladbare.
    textures: HashMap<String, TextureHandle>,
    broken: HashSet<String>,
    loaded: bool,
    items: Vec<ClipItem>,
    /// Aktueller Inhalt der System-Zwischenablage (nur zur Anzeige/Übernahme).
    system: String,
    last_read: f64,
    /// Text des neuen Eintrags – frei eingebbar oder aus der Zwischenablage übernommen.
    draft: String,
    title: String,
    category: String,
    /// `None` = alle Kategorien.
    filter: Option<String>,
    search: String,
    editing: Option<ClipItem>,
    /// Vom Nutzer festgelegte Reihenfolge der Kategorien; unbekannte folgen alphabetisch.
    order: Vec<String>,
    /// Zuletzt kopierter Eintrag (ID, Zeitpunkt) – die Zeile zeigt kurz „Kopiert“.
    copied: Option<(i64, f64)>,
}

/// Wie lange die Bestätigung „Kopiert“ in der Zeile stehen bleibt (Sekunden).
const COPIED_SECONDS: f64 = 1.6;
const NO_CATEGORY: &str = "Ohne Kategorie";

impl ClipboardView {
    /// Liest die Zwischenablage beim nächsten Bild neu.
    pub fn refresh(&mut self) {
        self.last_read = 0.0;
    }

    fn refresh_system(&mut self, ui: &Ui, shared: &Shared) {
        if shared.now - self.last_read >= REFRESH_SECONDS || self.last_read == 0.0 {
            self.last_read = shared.now;
            if let Ok(text) = clipboard::read_system() {
                self.system = text;
            }
        }
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(REFRESH_SECONDS));
    }

    fn save_draft(&mut self, shared: &mut Shared) {
        match clipboard::save(&shared.db, &self.title, &self.category, &self.draft) {
            Ok(item) => {
                self.items.insert(0, item);
                self.title.clear();
                self.draft.clear();
            }
            Err(err) => shared.notify(err),
        }
    }

    fn apply(&mut self, action: Action, shared: &mut Shared) {
        match action {
            Action::Copy(item) => {
                let result = match &item.image {
                    Some(name) => clipboard::load_image(&shared.data_dir, name)
                        .and_then(|image| clipboard::write_system_image(&image)),
                    None => clipboard::write_system(&item.content),
                };
                match result {
                    Ok(()) => {
                        // Rückmeldung direkt in der Zeile statt als Meldung am Fensterrand.
                        self.copied = Some((item.id, shared.now));
                        self.system = item.content;
                    }
                    Err(err) => shared.notify(err),
                }
            }
            Action::Edit(item) => self.editing = Some(item),
            Action::Delete(id) => match clipboard::delete(&shared.db, id) {
                Ok(()) => {
                    // Zu einem Bild-Eintrag gehört eine Datei, die mit verschwinden muss.
                    let image = self
                        .items
                        .iter()
                        .find(|item| item.id == id)
                        .and_then(|item| item.image.clone());
                    if let Some(name) = image {
                        clipboard::delete_image(&shared.data_dir, &name);
                        self.textures.remove(&name);
                    }
                    self.items.retain(|item| item.id != id);
                }
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
            Action::Move(id, category) => {
                let Some(mut moved) = self.items.iter().find(|item| item.id == id).cloned() else {
                    return;
                };
                if moved.category == category {
                    return;
                }
                moved.category = category;
                match clipboard::update(&shared.db, &moved) {
                    Ok(updated) => {
                        if let Some(slot) = self.items.iter_mut().find(|item| item.id == id) {
                            *slot = updated;
                        }
                    }
                    Err(err) => shared.notify(err),
                }
            }
            Action::Reorder(dragged, target) => {
                // Aktuelle Anzeigereihenfolge aller Kategorien, dann die gezogene vor das Ziel setzen.
                let names: BTreeSet<String> = self
                    .items
                    .iter()
                    .filter(|item| !item.category.is_empty())
                    .map(|item| item.category.clone())
                    .collect();
                let mut order: Vec<String> = names.into_iter().collect();
                order.sort_by_key(|name| self.order_index(name));
                order.retain(|name| name != &dragged);
                let position = order
                    .iter()
                    .position(|name| name == &target)
                    .unwrap_or(order.len());
                order.insert(position, dragged);
                self.order = order;
                let json = serde_json::to_string(&self.order).unwrap_or_else(|_| "[]".into());
                if let Err(err) = shared.db.set_setting(SETTING_CATEGORY_ORDER, &json) {
                    shared.notify(err);
                }
            }
        }
    }

    /// Position einer Kategorie in der gewählten Reihenfolge (unbekannte ans Ende).
    fn order_index(&self, name: &str) -> usize {
        self.order
            .iter()
            .position(|candidate| candidate == name)
            .unwrap_or(usize::MAX)
    }

    /// Eingabebereich: eigenen Text schreiben oder die Zwischenablage übernehmen.
    fn new_entry(&mut self, ui: &mut Ui, shared: &mut Shared) {
        let p = shared.palette;
        let mut save_image = false;
        theme::card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::card_title(ui, t("Neuer Eintrag"));
            ui.add(
                TextEdit::multiline(&mut self.draft)
                    .hint_text(t(
                        "Text hier eingeben – oder unten die Zwischenablage übernehmen",
                    ))
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            );

            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let can_save = !self.draft.trim().is_empty();
                    let clicked = theme::primary_button(ui, &p, t("Speichern"), can_save).clicked();
                    let category = ui.add(
                        TextEdit::singleline(&mut self.category)
                            .hint_text(t("Kategorie"))
                            .desired_width(170.0),
                    );
                    let title = ui.add(
                        TextEdit::singleline(&mut self.title)
                            .hint_text(t("Titel (optional)"))
                            .desired_width(f32::INFINITY),
                    );
                    let enter = (title.lost_focus() || category.lost_focus())
                        && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if can_save && (clicked || enter) {
                        self.save_draft(shared);
                    }
                });
            });

            // Aktuelle Zwischenablage als Vorschlag.
            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let has_text = !self.system.is_empty();
                    if ui
                        .add_enabled(has_text, egui::Button::new(t("Übernehmen")))
                        .clicked()
                    {
                        self.draft = self.system.clone();
                    }
                    if ui
                        .button(t("Bild speichern"))
                        .on_hover_text(t(
                            "Speichert ein kopiertes Bild oder einen Screenshot als Eintrag",
                        ))
                        .clicked()
                    {
                        save_image = true;
                    }
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        ui.label(theme::secondary(&p, t("Zwischenablage:")).size(12.5));
                        let preview = if has_text {
                            theme::first_line(&self.system, 140)
                        } else {
                            "enthält gerade keinen Text".to_owned()
                        };
                        ui.add(Label::new(theme::secondary(&p, preview).size(12.5)).truncate());
                    });
                });
            });
        });

        if save_image {
            match clipboard::read_system_image() {
                Ok(Some(image)) => {
                    match clipboard::save_image(
                        &shared.db,
                        &shared.data_dir,
                        &self.title,
                        &self.category,
                        &image,
                    ) {
                        Ok(item) => {
                            shared.notify(crate::i18n::td(format!("„{}“ gespeichert", item.title)));
                            self.items.insert(0, item);
                            self.title.clear();
                        }
                        Err(err) => shared.notify(err),
                    }
                }
                Ok(None) => shared.notify(t("In der Zwischenablage liegt kein Bild.")),
                Err(err) => shared.notify(err),
            }
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
            if let Ok(Some(json)) = shared.db.get_setting(SETTING_CATEGORY_ORDER) {
                self.order = serde_json::from_str(&json).unwrap_or_default();
            }
        }
        self.refresh_system(ui, shared);
        self.new_entry(ui, shared);

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
            if theme::chip(ui, &p, t("Alle"), self.filter.is_none()).clicked() {
                self.filter = None;
            }
            for name in &categories {
                if theme::chip(ui, &p, name, self.filter.as_ref() == Some(name)).clicked() {
                    self.filter = Some(name.clone());
                }
            }
            ui.add(
                TextEdit::singleline(&mut self.search)
                    .hint_text(t("Suchen"))
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
                "Noch nichts gespeichert. Gib oben einen Text ein und sichere ihn mit Titel und Kategorie."
            } else {
                "Keine Einträge für diesen Filter."
            };
            theme::empty_state(ui, &p, text);
            return;
        }

        ui.label(
            theme::secondary(
                &p,
                t("Klick kopiert · Rechtsklick für mehr · Einträge und Kategorien lassen sich ziehen"),
            )
            .size(12.0),
        );

        // Nach Kategorie gruppieren – jede Kategorie bekommt eine eigene Karte mit Überschrift.
        let mut groups: BTreeMap<String, Vec<ClipItem>> = BTreeMap::new();
        let mut uncategorized = Vec::new();
        for item in visible {
            if item.category.is_empty() {
                uncategorized.push(item);
            } else {
                groups.entry(item.category.clone()).or_default().push(item);
            }
        }
        let mut sections: Vec<(String, Vec<ClipItem>)> = groups.into_iter().collect();
        // Stabile Sortierung: gewählte Reihenfolge zuerst, der Rest bleibt alphabetisch.
        sections.sort_by_key(|(name, _)| self.order_index(name));
        if !uncategorized.is_empty() {
            sections.push((NO_CATEGORY.to_owned(), uncategorized));
        }

        let now = shared.now;
        let copied_id = self
            .copied
            .filter(|(_, at)| now - at < COPIED_SECONDS)
            .map(|(id, _)| id);
        if copied_id.is_some() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(200));
        }

        // Vorschaubilder laden – höchstens zwei je Frame, damit nichts ruckelt.
        let missing: Vec<String> = self
            .items
            .iter()
            .filter_map(|item| item.image.clone())
            .filter(|name| !self.textures.contains_key(name) && !self.broken.contains(name))
            .take(2)
            .collect();
        for name in missing {
            match clipboard::load_image(&shared.data_dir, &name) {
                Ok(image) => {
                    let small = image::DynamicImage::ImageRgba8(image)
                        .thumbnail(256, 256)
                        .to_rgba8();
                    let color_image = egui::ColorImage::from_rgba_unmultiplied(
                        [small.width() as usize, small.height() as usize],
                        small.as_raw(),
                    );
                    let texture = ui.ctx().load_texture(
                        format!("clip-image:{name}"),
                        color_image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.textures.insert(name, texture);
                    ui.ctx().request_repaint();
                }
                Err(_) => {
                    self.broken.insert(name);
                }
            }
        }

        let mut action = None;
        for (category, items) in sections {
            ui.add_space(4.0);
            // Kopfzeile über die ganze Breite: Ziehgriff der Kategorie und Ablageziel.
            let is_real_category = category != NO_CATEGORY;
            let target_category = if is_real_category {
                category.clone()
            } else {
                String::new()
            };
            let header_rect =
                Rect::from_min_size(ui.cursor().min, egui::vec2(ui.available_width(), 24.0));
            let header = ui.interact(
                header_rect,
                ui.id().with(("clip-category", category.as_str())),
                Sense::drag(),
            );
            if is_real_category {
                header.dnd_set_drag_payload(ClipDrag::Category(category.clone()));
            }
            if header.dnd_hover_payload::<ClipDrag>().is_some() {
                theme::drop_highlight(ui, &p, header_rect);
            }
            if let Some(payload) = header.dnd_release_payload::<ClipDrag>() {
                match &*payload {
                    ClipDrag::Item(id) => {
                        action = Some(Action::Move(*id, target_category.clone()));
                    }
                    ClipDrag::Category(dragged) if is_real_category && dragged != &category => {
                        action = Some(Action::Reorder(dragged.clone(), category.clone()));
                    }
                    ClipDrag::Category(_) => {}
                }
            }
            ui.allocate_ui_with_layout(
                header_rect.size(),
                Layout::left_to_right(Align::Center),
                |ui| {
                    if is_real_category {
                        theme::badge(ui, &p, &category);
                    } else {
                        ui.label(theme::secondary(&p, category.as_str()).strong());
                    }
                    ui.label(
                        theme::secondary(&p, crate::i18n::td(format!("{} Einträge", items.len())))
                            .size(12.0),
                    );
                },
            );

            theme::card(&p).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 2.0;
                for item in items {
                    match self.editing.as_mut().filter(|draft| draft.id == item.id) {
                        Some(draft) => {
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                ui.add(
                                    TextEdit::singleline(&mut draft.title)
                                        .hint_text(t("Titel"))
                                        .desired_width(260.0),
                                );
                                ui.add(
                                    TextEdit::singleline(&mut draft.category)
                                        .hint_text(t("Kategorie"))
                                        .desired_width(170.0),
                                );
                            });
                            ui.add(
                                TextEdit::multiline(&mut draft.content)
                                    .desired_rows(4)
                                    .desired_width(f32::INFINITY),
                            );
                            ui.horizontal(|ui| {
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if theme::primary_button(ui, &p, t("Sichern"), true).clicked() {
                                        action = Some(Action::SaveEdit);
                                    }
                                    if ui.button(t("Abbrechen")).clicked() {
                                        action = Some(Action::CancelEdit);
                                    }
                                });
                            });
                            ui.add_space(6.0);
                        }
                        None => {
                            let just_copied = copied_id == Some(item.id);
                            let thumbnail = item
                                .image
                                .as_ref()
                                .and_then(|name| self.textures.get(name))
                                .cloned();
                            let row = theme::clickable_row(ui, &p, ROW_HEIGHT, just_copied, |ui| {
                                if just_copied {
                                    ui.label(RichText::new(t("Kopiert")).color(p.success).strong());
                                } else {
                                    if ui.small_button(t("Löschen")).clicked() {
                                        action = Some(Action::Delete(item.id));
                                    }
                                    if ui.small_button(t("Bearbeiten")).clicked() {
                                        action = Some(Action::Edit(item.clone()));
                                    }
                                }
                                // Restbreite: Vorschaubild (bei Bildern), Titel und Inhalt.
                                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                                    if let Some(texture) = &thumbnail {
                                        let size = texture.size_vec2();
                                        let scale = ((ROW_HEIGHT - 8.0) / size.y.max(1.0))
                                            .min(110.0 / size.x.max(1.0));
                                        let source = egui::load::SizedTexture::new(
                                            texture.id(),
                                            size * scale,
                                        );
                                        ui.add(egui::Image::new(source).rounding(p.radius));
                                    }
                                    ui.with_layout(Layout::top_down(Align::Min), |ui| {
                                        ui.spacing_mut().item_spacing.y = 1.0;
                                        ui.add_space(4.0);
                                        ui.add(
                                            Label::new(RichText::new(&item.title).strong())
                                                .truncate(),
                                        );
                                        let preview = if item.image.is_some() {
                                            "Bild – Klick kopiert es".to_owned()
                                        } else {
                                            theme::first_line(&item.content, 160)
                                        };
                                        ui.add(
                                            Label::new(theme::secondary(&p, preview).size(12.5))
                                                .truncate(),
                                        );
                                    });
                                });
                            });
                            if row.clicked() {
                                action = Some(Action::Copy(item.clone()));
                            }
                            // Ziehen: Eintrag in eine andere Kategorie legen (auf deren
                            // Kopfzeile oder auf einen ihrer Einträge).
                            row.dnd_set_drag_payload(ClipDrag::Item(item.id));
                            if let Some(payload) = row.dnd_release_payload::<ClipDrag>() {
                                if let ClipDrag::Item(id) = &*payload {
                                    if *id != item.id {
                                        action = Some(Action::Move(*id, item.category.clone()));
                                    }
                                }
                            }
                            row.context_menu(|ui| {
                                if ui.button(t("Kopieren")).clicked() {
                                    action = Some(Action::Copy(item.clone()));
                                    ui.close_menu();
                                }
                                if ui.button(t("Bearbeiten")).clicked() {
                                    action = Some(Action::Edit(item.clone()));
                                    ui.close_menu();
                                }
                                ui.separator();
                                if ui
                                    .button(RichText::new(t("Löschen")).color(p.danger))
                                    .clicked()
                                {
                                    action = Some(Action::Delete(item.id));
                                    ui.close_menu();
                                }
                            });
                        }
                    }
                }
            });
        }

        if let Some(action) = action {
            self.apply(action, shared);
        }
    }
}
