use crate::i18n::t;
use std::collections::{BTreeMap, BTreeSet, HashMap};

use eframe::egui::{
    self, Align, CentralPanel, CollapsingHeader, Frame, Layout, Margin, RichText, ScrollArea,
    Sense, SidePanel, TextEdit, TextureHandle, Ui,
};

use crate::{
    app::Shared,
    services::{
        clipboard,
        note_blocks::{self, Block},
        notes::{now_ms, ExportFormat, Note},
    },
    theme::{self, Palette},
};

/// Breiter werden Bilder im Text nicht dargestellt.
const INLINE_IMAGE_MAX_WIDTH: f32 = 560.0;
/// Größte Kantenlänge, mit der Bilder als Textur geladen werden.
const MAX_TEXTURE_SIDE: u32 = 1600;
const IMAGE_EXTENSIONS: [&str; 6] = ["png", "jpg", "jpeg", "bmp", "gif", "webp"];

/// Was der Nutzer in Liste, Editor oder Kontextmenü ausgelöst hat (wird nach dem Zeichnen ausgeführt).
enum Action {
    Select(String),
    /// Neue Notiz in der angegebenen Kategorie.
    New(String),
    Duplicate(String),
    /// Notiz (ID) in eine Kategorie verschieben.
    Move(String, String),
    Delete(String),
    /// Bilddateien auswählen und an die geöffnete Notiz hängen.
    AddImageFiles,
    PasteImage,
    RemoveImage(String),
    Export(ExportFormat),
}

/// Nutzlast beim Ziehen einer Notiz (ihre ID).
struct NoteDrag(String);

/// Ein Ordner im Kategoriebaum.
#[derive(Default)]
struct Folder {
    children: BTreeMap<String, Folder>,
    /// Indizes in die Notizliste.
    notes: Vec<usize>,
}

/// Zerlegt „Arbeit / Projekt A“ in die Ebenen `Arbeit`, `Projekt A`.
fn path_parts(category: &str) -> impl Iterator<Item = &str> {
    category
        .split('/')
        .map(str::trim)
        .filter(|part| !part.is_empty())
}

fn normalized(category: &str) -> String {
    path_parts(category).collect::<Vec<_>>().join("/")
}

fn build_tree(notes: &[Note]) -> Folder {
    let mut root = Folder::default();
    for (index, note) in notes.iter().enumerate() {
        let mut folder = &mut root;
        for part in path_parts(&note.category) {
            folder = folder.children.entry(part.to_owned()).or_default();
        }
        folder.notes.push(index);
    }
    root
}

fn display_title(note: &Note) -> &str {
    if note.title.trim().is_empty() {
        "Neue Notiz"
    } else {
        note.title.as_str()
    }
}

/// Dateiname aus dem Titel: Zeichen, die Windows in Dateinamen verbietet, werden ersetzt.
fn file_stem(note: &Note) -> String {
    let cleaned: String = display_title(note)
        .chars()
        .map(|c| {
            if "\\/:*?\"<>|".contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.');
    if trimmed.is_empty() {
        "Notiz".to_owned()
    } else {
        trimmed.chars().take(80).collect()
    }
}

/// Kontextmenü einer Notiz (Rechtsklick in der Liste).
fn note_menu(
    ui: &mut Ui,
    p: &Palette,
    note: &Note,
    categories: &BTreeSet<String>,
    actions: &mut Vec<Action>,
) {
    if ui.button(t("Öffnen")).clicked() {
        actions.push(Action::Select(note.id.clone()));
        ui.close_menu();
    }
    if ui.button(t("Duplizieren")).clicked() {
        actions.push(Action::Duplicate(note.id.clone()));
        ui.close_menu();
    }
    ui.menu_button(t("Verschieben nach"), |ui| {
        if ui.button(t("Ohne Kategorie")).clicked() {
            actions.push(Action::Move(note.id.clone(), String::new()));
            ui.close_menu();
        }
        for category in categories {
            if ui.button(category.as_str()).clicked() {
                actions.push(Action::Move(note.id.clone(), category.clone()));
                ui.close_menu();
            }
        }
    });
    if ui.button(t("Neue Notiz in dieser Kategorie")).clicked() {
        actions.push(Action::New(normalized(&note.category)));
        ui.close_menu();
    }
    ui.separator();
    if ui
        .button(RichText::new(t("Löschen")).color(p.danger))
        .clicked()
    {
        actions.push(Action::Delete(note.id.clone()));
        ui.close_menu();
    }
}

/// Zeichnet einen Ordner samt Unterordnern (rekursiv). Notizen lassen sich auf
/// Ordner und auf andere Notizen ziehen, um sie in deren Kategorie zu legen.
#[allow(clippy::too_many_arguments)]
fn show_folder(
    ui: &mut Ui,
    p: &Palette,
    folder: &Folder,
    path: &str,
    notes: &[Note],
    active: Option<&str>,
    categories: &BTreeSet<String>,
    actions: &mut Vec<Action>,
) {
    for (name, child) in &folder.children {
        let child_path = if path.is_empty() {
            name.clone()
        } else {
            format!("{path}/{name}")
        };
        let header = CollapsingHeader::new(RichText::new(name).strong())
            .id_salt(&child_path)
            .default_open(true)
            .show(ui, |ui| {
                show_folder(
                    ui,
                    p,
                    child,
                    &child_path,
                    notes,
                    active,
                    categories,
                    actions,
                );
            });
        let header = header.header_response;
        if header.dnd_hover_payload::<NoteDrag>().is_some() {
            theme::drop_highlight(ui, p, header.rect);
        }
        if let Some(dragged) = header.dnd_release_payload::<NoteDrag>() {
            actions.push(Action::Move(dragged.0.clone(), child_path.clone()));
        }
        header.context_menu(|ui| {
            if ui.button(t("Neue Notiz hier")).clicked() {
                actions.push(Action::New(child_path.clone()));
                ui.close_menu();
            }
        });
    }

    for &index in &folder.notes {
        let note = &notes[index];
        let row = theme::nav_item(ui, p, display_title(note), active == Some(note.id.as_str()))
            .on_hover_text(theme::first_line(&note.body, 80));
        if row.clicked() {
            actions.push(Action::Select(note.id.clone()));
        }
        row.dnd_set_drag_payload(NoteDrag(note.id.clone()));
        if let Some(dragged) = row.dnd_release_payload::<NoteDrag>() {
            if dragged.0 != note.id {
                actions.push(Action::Move(dragged.0.clone(), path.to_owned()));
            }
        }
        row.context_menu(|ui| note_menu(ui, p, note, categories, actions));
    }
}

#[derive(Default)]
pub struct NotesView {
    loaded: bool,
    notes: Vec<Note>,
    active: Option<String>,
    /// Geladene Bild-Texturen je Dateiname.
    textures: HashMap<String, TextureHandle>,
    /// Bild, das gerade groß angezeigt wird.
    preview: Option<String>,
    /// Schreibmarke: Notiz-ID, Textblock und Zeichenposition – dort landen neue Bilder.
    cursor: Option<(String, usize, usize)>,
}

impl NotesView {
    fn active_note_mut(&mut self) -> Option<&mut Note> {
        let active = self.active.as_ref()?;
        self.notes.iter_mut().find(|note| &note.id == active)
    }

    /// Liefert die Textur eines Bildes und lädt sie beim ersten Zugriff.
    fn texture(&mut self, ui: &Ui, shared: &Shared, name: &str) -> Option<TextureHandle> {
        if let Some(texture) = self.textures.get(name) {
            return Some(texture.clone());
        }
        let mut image = image::DynamicImage::ImageRgba8(shared.notes.load_image(name).ok()?);
        if image.width().max(image.height()) > MAX_TEXTURE_SIDE {
            image = image.thumbnail(MAX_TEXTURE_SIDE, MAX_TEXTURE_SIDE);
        }
        let rgba = image.to_rgba8();
        let color_image = egui::ColorImage::from_rgba_unmultiplied(
            [rgba.width() as usize, rgba.height() as usize],
            rgba.as_raw(),
        );
        let texture = ui.ctx().load_texture(
            format!("note-image:{name}"),
            color_image,
            egui::TextureOptions::LINEAR,
        );
        self.textures.insert(name.to_owned(), texture.clone());
        Some(texture)
    }

    /// Hängt gespeicherte Bilder an die geöffnete Notiz und sichert sie.
    fn attach_images(&mut self, names: Vec<String>, shared: &mut Shared) {
        if names.is_empty() {
            return;
        }
        let cursor = self.cursor.take();
        let Some(note) = self.active_note_mut() else {
            return;
        };
        // An der Schreibmarke einfügen, sofern sie zu dieser Notiz gehört – sonst am Ende.
        let position = cursor
            .filter(|(note_id, _, _)| note_id == &note.id)
            .map(|(_, block_index, char_index)| (block_index, char_index));
        note.body = note_blocks::insert_images(&note.body, position, &names);
        note.images.extend(names);
        note.updated_at = now_ms();
        if let Err(err) = shared.notes.save(note) {
            shared.notify(err);
        }
    }

    fn apply(&mut self, action: Action, shared: &mut Shared) {
        match action {
            Action::Select(id) => self.active = Some(id),
            Action::New(category) => match shared.notes.create(&category, "", "") {
                Ok(note) => {
                    self.active = Some(note.id.clone());
                    self.notes.insert(0, note);
                }
                Err(err) => shared.notify(err),
            },
            Action::Duplicate(id) => {
                let Some(source) = self.notes.iter().find(|note| note.id == id) else {
                    return;
                };
                let title = crate::i18n::td(format!("{} (Kopie)", display_title(source)));
                match shared.notes.create(&source.category, &title, &source.body) {
                    Ok(note) => {
                        self.active = Some(note.id.clone());
                        self.notes.insert(0, note);
                    }
                    Err(err) => shared.notify(err),
                }
            }
            Action::Move(id, category) => {
                if let Some(note) = self.notes.iter_mut().find(|note| note.id == id) {
                    if normalized(&note.category) == category {
                        return;
                    }
                    note.category = category;
                    note.updated_at = now_ms();
                    if let Err(err) = shared.notes.save(note) {
                        shared.notify(err);
                    }
                }
            }
            Action::Delete(id) => match shared.notes.delete(&id) {
                Ok(()) => {
                    // Die Bilder der Notiz mit entfernen.
                    if let Some(note) = self.notes.iter().find(|note| note.id == id) {
                        for name in &note.images {
                            shared.notes.delete_image(name);
                            self.textures.remove(name);
                        }
                    }
                    self.notes.retain(|note| note.id != id);
                    if self.active.as_deref() == Some(id.as_str()) {
                        self.active = self.notes.first().map(|note| note.id.clone());
                    }
                }
                Err(err) => shared.notify(err),
            },
            Action::AddImageFiles => {
                let Some(paths) = rfd::FileDialog::new()
                    .set_title(t("Bilder hinzufügen"))
                    .add_filter("Bilder", &IMAGE_EXTENSIONS)
                    .pick_files()
                else {
                    return;
                };
                let mut names = Vec::new();
                for path in paths {
                    match shared.notes.add_image_file(&path) {
                        Ok(name) => names.push(name),
                        Err(err) => shared.notify(err),
                    }
                }
                self.attach_images(names, shared);
            }
            Action::PasteImage => match clipboard::read_system_image() {
                Ok(Some(image)) => match shared.notes.add_image(&image) {
                    Ok(name) => self.attach_images(vec![name], shared),
                    Err(err) => shared.notify(err),
                },
                Ok(None) => shared.notify(t("In der Zwischenablage liegt kein Bild.")),
                Err(err) => shared.notify(err),
            },
            Action::RemoveImage(name) => {
                if let Some(note) = self.active_note_mut() {
                    note.body = note_blocks::remove_image(&note.body, &name);
                    note.images.retain(|image| image != &name);
                    note.updated_at = now_ms();
                    if let Err(err) = shared.notes.save(note) {
                        shared.notify(err);
                    }
                }
                shared.notes.delete_image(&name);
                self.textures.remove(&name);
                if self.preview.as_deref() == Some(name.as_str()) {
                    self.preview = None;
                }
            }
            Action::Export(format) => {
                let Some(note) = self.active_note_mut().map(|note| note.clone()) else {
                    return;
                };
                let Some(path) = rfd::FileDialog::new()
                    .set_title(t("Notiz exportieren"))
                    .set_file_name(format!("{}.{}", file_stem(&note), format.extension()))
                    .add_filter(format.label(), &[format.extension()])
                    .save_file()
                else {
                    return;
                };
                match shared.notes.export(&note, &path, format) {
                    Ok(()) => shared.notify(crate::i18n::td(format!(
                        "Exportiert nach {}",
                        path.display()
                    ))),
                    Err(err) => shared.notify(err),
                }
            }
        }
    }

    /// Großansicht eines Bildes in einem eigenen kleinen Fenster.
    fn preview_window(&mut self, ui: &Ui, shared: &Shared) {
        let Some(name) = self.preview.clone() else {
            return;
        };
        let Some(texture) = self.texture(ui, shared, &name) else {
            self.preview = None;
            return;
        };
        let mut open = true;
        let screen = ui.ctx().screen_rect().size();
        let size = texture.size_vec2();
        // Auf höchstens 80 % des Fensters verkleinern, nie vergrößern.
        let scale = (screen.x * 0.8 / size.x)
            .min(screen.y * 0.75 / size.y)
            .min(1.0);
        egui::Window::new("Bild")
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ui.ctx(), |ui| {
                let source = egui::load::SizedTexture::new(texture.id(), size * scale);
                ui.add(egui::Image::new(source));
            });
        if !open {
            self.preview = None;
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, shared: &mut Shared) {
        let p = shared.palette;
        if !self.loaded {
            self.loaded = true;
            match shared.notes.list() {
                Ok(mut notes) => {
                    // Ältere Notizen führten Bilder als Anhang – sie wandern ans Textende.
                    for note in &mut notes {
                        let embedded = note_blocks::image_names(&note.body);
                        let loose: Vec<String> = note
                            .images
                            .iter()
                            .filter(|name| !embedded.contains(name))
                            .cloned()
                            .collect();
                        if !loose.is_empty() {
                            note.body = note_blocks::insert_images(&note.body, None, &loose);
                            let _ = shared.notes.save(note);
                        }
                    }
                    self.active = notes.first().map(|note| note.id.clone());
                    self.notes = notes;
                }
                Err(err) => shared.notify(err),
            }
        }

        let mut actions = Vec::new();
        let categories: BTreeSet<String> = self
            .notes
            .iter()
            .map(|note| normalized(&note.category))
            .filter(|category| !category.is_empty())
            .collect();

        // ── Linke Spalte: Kategoriebaum mit Notizen ──
        SidePanel::left("notes_list")
            .exact_width(240.0)
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
                        theme::secondary(
                            &p,
                            crate::i18n::td(format!("{} Notizen", self.notes.len())),
                        )
                        .size(12.0),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button(t("Neue Notiz")).clicked() {
                            // Neue Notizen landen in der Kategorie der gerade geöffneten Notiz.
                            let category = self
                                .notes
                                .iter()
                                .find(|note| Some(&note.id) == self.active.as_ref())
                                .map(|note| normalized(&note.category))
                                .unwrap_or_default();
                            actions.push(Action::New(category));
                        }
                    });
                });
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        let tree = build_tree(&self.notes);
                        show_folder(
                            ui,
                            &p,
                            &tree,
                            "",
                            &self.notes,
                            self.active.as_deref(),
                            &categories,
                            &mut actions,
                        );

                        // Freie Fläche darunter: Rechtsklick legt eine Notiz an,
                        // Ablegen nimmt eine Notiz aus ihrer Kategorie heraus.
                        let rest = ui.available_size();
                        let height = if rest.y.is_finite() {
                            rest.y.clamp(40.0, 2000.0)
                        } else {
                            40.0
                        };
                        let (_, empty) = ui.allocate_exact_size(
                            egui::vec2(rest.x.max(10.0), height),
                            Sense::click(),
                        );
                        if let Some(dragged) = empty.dnd_release_payload::<NoteDrag>() {
                            actions.push(Action::Move(dragged.0.clone(), String::new()));
                        }
                        empty.context_menu(|ui| {
                            if ui.button(t("Neue Notiz")).clicked() {
                                actions.push(Action::New(String::new()));
                                ui.close_menu();
                            }
                        });
                    });
            });

        // ── Rechte Spalte: Editor ──
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
                let Some(index) = self
                    .notes
                    .iter()
                    .position(|note| Some(&note.id) == active_id.as_ref())
                else {
                    theme::empty_state(
                        ui,
                        &p,
                        t("Keine Notiz ausgewählt. Lege links eine neue an."),
                    );
                    return;
                };

                // Texturen der eingebetteten Bilder vorab laden – danach wird die Notiz
                // veränderlich ausgeliehen.
                let mut blocks = note_blocks::parse(&self.notes[index].body);
                let mut loaded: HashMap<String, TextureHandle> = HashMap::new();
                for block in &blocks {
                    if let Block::Image(name) = block {
                        if let Some(texture) = self.texture(ui, shared, name) {
                            loaded.insert(name.clone(), texture);
                        }
                    }
                }

                let note = &mut self.notes[index];
                let mut changed = false;
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .button(RichText::new(t("Löschen")).color(p.danger))
                            .clicked()
                        {
                            actions.push(Action::Delete(note.id.clone()));
                        }
                        ui.menu_button(t("Exportieren"), |ui| {
                            for format in ExportFormat::ALL {
                                if ui.button(format.label()).clicked() {
                                    actions.push(Action::Export(format));
                                    ui.close_menu();
                                }
                            }
                        });
                        changed |= ui
                            .add(
                                TextEdit::singleline(&mut note.title)
                                    .hint_text(t("Titel"))
                                    .font(egui::TextStyle::Heading)
                                    .frame(false)
                                    .desired_width(f32::INFINITY),
                            )
                            .changed();
                    });
                });
                ui.horizontal(|ui| {
                    ui.label(theme::secondary(&p, t("Kategorie")).size(12.5));
                    changed |= ui
                        .add(
                            TextEdit::singleline(&mut note.category)
                                .hint_text(t("z. B. Arbeit/Projekt A – „/“ trennt Unterkategorien"))
                                .desired_width(f32::INFINITY),
                        )
                        .changed();
                });
                ui.horizontal(|ui| {
                    if ui.small_button(t("Bild einfügen …")).clicked() {
                        actions.push(Action::AddImageFiles);
                    }
                    if ui
                        .small_button(t("Bild aus Zwischenablage"))
                        .on_hover_text(t("Fügt ein kopiertes Bild oder einen Screenshot ein"))
                        .clicked()
                    {
                        actions.push(Action::PasteImage);
                    }
                    ui.label(
                        theme::secondary(&p, t("Bilder landen an der Schreibmarke.")).size(12.0),
                    );
                });
                ui.separator();

                let note_id = note.id.clone();
                let block_count = blocks.len();
                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        for (block_index, block) in blocks.iter_mut().enumerate() {
                            match block {
                                Block::Text(text) => {
                                    // Der letzte Textblock ist höher, damit unter dem Inhalt Platz zum Klicken bleibt.
                                    let is_last = block_index + 1 == block_count;
                                    let mut edit = TextEdit::multiline(text)
                                        .id_salt(("note-block", &note_id, block_index))
                                        .frame(false)
                                        .desired_rows(if is_last { 10 } else { 1 })
                                        .desired_width(f32::INFINITY);
                                    if block_count == 1 {
                                        edit = edit.hint_text(t("Schreib los …"));
                                    }
                                    let output = edit.show(ui);
                                    changed |= output.response.changed();
                                    if output.response.has_focus() {
                                        if let Some(range) = output.cursor_range {
                                            self.cursor = Some((
                                                note_id.clone(),
                                                block_index,
                                                range.primary.ccursor.index,
                                            ));
                                        }
                                    }

                                    // Rechtsklick im Text.
                                    output.response.context_menu(|ui| {
                                        if ui.button(t("Text kopieren")).clicked() {
                                            ui.ctx().copy_text(text.clone());
                                            ui.close_menu();
                                        }
                                        if ui.button(t("Zwischenablage anhängen")).clicked() {
                                            if let Ok(pasted) = clipboard::read_system() {
                                                if !text.is_empty() && !text.ends_with('\n') {
                                                    text.push('\n');
                                                }
                                                text.push_str(&pasted);
                                                changed = true;
                                            }
                                            ui.close_menu();
                                        }
                                        if ui
                                            .button(t("Bild aus Zwischenablage einfügen"))
                                            .clicked()
                                        {
                                            actions.push(Action::PasteImage);
                                            ui.close_menu();
                                        }
                                        ui.separator();
                                        if ui
                                            .button(RichText::new(t("Text leeren")).color(p.danger))
                                            .clicked()
                                        {
                                            text.clear();
                                            changed = true;
                                            ui.close_menu();
                                        }
                                    });
                                }
                                Block::Image(name) => {
                                    let Some(texture) = loaded.get(name.as_str()) else {
                                        ui.label(theme::secondary(&p, t("Bild fehlt")));
                                        continue;
                                    };
                                    let size = texture.size_vec2();
                                    let max_width =
                                        ui.available_width().min(INLINE_IMAGE_MAX_WIDTH);
                                    let scale = (max_width / size.x.max(1.0)).min(1.0);
                                    let source =
                                        egui::load::SizedTexture::new(texture.id(), size * scale);
                                    let picture = ui
                                        .add(
                                            egui::Image::new(source)
                                                .rounding(p.radius)
                                                .sense(Sense::click()),
                                        )
                                        .on_hover_text(t(
                                            "Klick vergrößert · Rechtsklick entfernt",
                                        ));
                                    if picture.clicked() {
                                        self.preview = Some(name.clone());
                                    }
                                    picture.context_menu(|ui| {
                                        if ui.button(t("Vergrößern")).clicked() {
                                            self.preview = Some(name.clone());
                                            ui.close_menu();
                                        }
                                        if ui
                                            .button(
                                                RichText::new(t("Bild entfernen")).color(p.danger),
                                            )
                                            .clicked()
                                        {
                                            actions.push(Action::RemoveImage(name.clone()));
                                            ui.close_menu();
                                        }
                                    });
                                }
                            }
                        }
                    });

                // Jede Änderung sofort auf die Platte schreiben – die Dateien sind winzig.
                if changed {
                    note.body = note_blocks::join(&blocks);
                    note.updated_at = now_ms();
                    if let Err(err) = shared.notes.save(note) {
                        save_error = Some(err);
                    }
                }
            });

        self.preview_window(ui, shared);

        if let Some(err) = save_error {
            shared.notify(err);
        }
        for action in actions {
            self.apply(action, shared);
        }
    }
}
