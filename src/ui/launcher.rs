use crate::i18n::t;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, TryRecvError},
        Arc,
    },
    thread,
};

use eframe::egui::{
    self, Align, Button, CollapsingHeader, Label, Layout, RichText, TextEdit, TextureHandle, Ui,
};

use crate::{
    app::Shared,
    services::{
        icons::{self, IconImage},
        launcher::{self, AppEntry},
    },
    theme::{self, Palette},
};

const SETTING_SELECTION: &str = "launcher_selection";
const SETTING_HIDDEN: &str = "launcher_hidden";
const COLUMNS: usize = 3;
const ROW_HEIGHT: f32 = 38.0;
const ICON_SIDE: f32 = 24.0;
/// Anzahl der Threads, die Icons laden.
const ICON_THREADS: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Filter {
    #[default]
    All,
    Selected,
    Hidden,
}

enum Action {
    Launch(String),
    ToggleSelected(String),
    Hide(String),
    Unhide(String),
    ToggleFavorite(String),
    /// Selbst hinzugefügten Eintrag wieder entfernen.
    RemoveCustom(String),
    /// Nachfragen, bevor ein eigener Eintrag entfernt wird.
    AskRemoveCustom(String),
}

/// Ordnername, unter dem selbst hinzugefügte Einträge erscheinen.
const CUSTOM_FOLDER: &str = "Eigene Einträge";

/// Nachrichten des Scan-Threads: erst die (schnelle) Liste, danach die Icons einzeln.
enum ScanMessage {
    Apps(Vec<AppEntry>),
    Icon(String, IconImage),
}

#[derive(Default)]
pub struct LauncherView {
    /// Wartet auf Bestätigung: eigenen Eintrag (Pfad) entfernen.
    confirm_remove: Option<String>,
    /// Kopien aus den Einstellungen: eigene Einträge (Name, Pfad) und Favoriten.
    custom: Vec<(String, String)>,
    favorites: BTreeSet<String>,
    /// Eingabefeld für den Namen einer neuen Sammlung.
    collection_name: String,
    settings_loaded: bool,
    apps: Vec<AppEntry>,
    /// Icon-Texturen je App-Pfad.
    textures: HashMap<String, TextureHandle>,
    /// Läuft gerade ein Scan, liefert dieser Kanal die Ergebnisse.
    scan: Option<Receiver<ScanMessage>>,
    search: String,
    filter: Filter,
    /// Auswahl (Pfade) – bleibt gespeichert, um dasselbe App-Set erneut zu starten.
    selected: BTreeSet<String>,
    /// Vom Nutzer ausgeblendete Apps (Pfade).
    hidden: BTreeSet<String>,
}

impl LauncherView {
    /// Sucht die Programme neu, sofern nicht schon ein Scan läuft.
    pub fn refresh(&mut self, ctx: &egui::Context) {
        if self.scan.is_none() {
            self.start_scan(ctx);
        }
    }

    /// Sucht die Programme im Hintergrund. Die Liste selbst kommt nach wenigen
    /// Millisekunden, die Icons werden danach Stück für Stück nachgereicht.
    pub fn start_scan(&mut self, ctx: &egui::Context) {
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let custom = self.custom.clone();
        thread::spawn(move || {
            let mut apps = launcher::scan();
            // Eigene Einträge gehören dazu wie installierte Programme.
            for (name, path) in custom {
                apps.push(AppEntry {
                    name,
                    path,
                    folder: CUSTOM_FOLDER.to_owned(),
                    system: false,
                });
            }
            // Icons der normalen Programme zuerst, Systemwerkzeuge zuletzt.
            let mut paths: Vec<(bool, String)> = apps
                .iter()
                .map(|app| (app.system, app.path.clone()))
                .collect();
            paths.sort_by_key(|(system, _)| *system);

            if tx.send(ScanMessage::Apps(apps)).is_err() {
                return;
            }
            ctx.request_repaint();

            // Icons parallel laden: mehrere Threads holen sich der Reihe nach den nächsten Pfad.
            let paths = Arc::new(paths);
            let next = Arc::new(AtomicUsize::new(0));
            for _ in 0..ICON_THREADS {
                let (paths, next, tx, ctx) = (
                    Arc::clone(&paths),
                    Arc::clone(&next),
                    tx.clone(),
                    ctx.clone(),
                );
                thread::spawn(move || {
                    let _com = icons::ComGuard::new();
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some((_, path)) = paths.get(index) else {
                            break;
                        };
                        if let Some(icon) = icons::extract(Path::new(path)) {
                            if tx.send(ScanMessage::Icon(path.clone(), icon)).is_err() {
                                return; // Ansicht wurde geschlossen oder neu gescannt
                            }
                        }
                        if index % 12 == 0 {
                            ctx.request_repaint();
                        }
                    }
                    ctx.request_repaint();
                });
            }
        });
        self.scan = Some(rx);
    }

    /// Übernimmt eingetroffene Scan-Ergebnisse und lädt Icons als Texturen.
    fn receive_scan(&mut self, ui: &Ui) {
        let Some(rx) = &self.scan else {
            return;
        };
        let mut finished = false;
        loop {
            match rx.try_recv() {
                Ok(ScanMessage::Apps(apps)) => {
                    // Texturen verschwundener Apps freigeben, vorhandene behalten.
                    let paths: BTreeSet<&str> = apps.iter().map(|app| app.path.as_str()).collect();
                    self.textures
                        .retain(|path, _| paths.contains(path.as_str()));
                    self.apps = apps;
                }
                Ok(ScanMessage::Icon(path, icon)) => {
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [icon.width, icon.height],
                        &icon.rgba,
                    );
                    let texture = ui.ctx().load_texture(
                        format!("app-icon:{path}"),
                        image,
                        egui::TextureOptions::LINEAR,
                    );
                    self.textures.insert(path, texture);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    finished = true;
                    break;
                }
            }
        }
        if finished {
            self.scan = None;
        }
    }

    fn load_settings(&mut self, shared: &Shared) {
        let load = |key: &str| -> BTreeSet<String> {
            shared
                .db
                .get_setting(key)
                .ok()
                .flatten()
                .and_then(|json| serde_json::from_str(&json).ok())
                .unwrap_or_default()
        };
        self.selected = load(SETTING_SELECTION);
        self.hidden = load(SETTING_HIDDEN);
    }

    fn persist(&self, key: &str, set: &BTreeSet<String>, shared: &mut Shared) {
        let json = serde_json::to_string(set).unwrap_or_else(|_| "[]".into());
        if let Err(err) = shared.db.set_setting(key, &json) {
            shared.notify(err);
        }
    }

    fn launch(&self, paths: &[String], shared: &mut Shared) {
        let custom: Vec<String> = self.custom.iter().map(|(_, path)| path.clone()).collect();
        let (started, error) = launcher::launch(paths, &custom);
        match error {
            // Ein einzelner Start braucht keine Meldung – die App erscheint ja.
            None if started <= 1 => {}
            None => shared.notify(crate::i18n::td(format!("{started} Apps gestartet"))),
            Some(error) => shared.notify(crate::i18n::td(format!(
                "{} von {} fehlgeschlagen: {error}",
                paths.len() - started,
                paths.len()
            ))),
        }
    }

    fn apply(&mut self, action: Action, shared: &mut Shared) {
        match action {
            Action::Launch(path) => self.launch(&[path], shared),
            Action::ToggleFavorite(path) => {
                let favorites = &mut shared.settings.launcher_favorites;
                if favorites.contains(&path) {
                    favorites.retain(|favorite| favorite != &path);
                } else {
                    favorites.push(path);
                }
                shared.save_settings();
            }
            Action::AskRemoveCustom(path) => {
                self.confirm_remove = Some(path);
            }
            Action::RemoveCustom(path) => {
                self.confirm_remove = None;
                shared
                    .settings
                    .launcher_custom
                    .retain(|(_, custom)| custom != &path);
                shared
                    .settings
                    .launcher_favorites
                    .retain(|favorite| favorite != &path);
                shared.save_settings();
            }
            Action::ToggleSelected(path) => {
                if !self.selected.remove(&path) {
                    self.selected.insert(path);
                }
                self.persist(SETTING_SELECTION, &self.selected, shared);
            }
            Action::Hide(path) => {
                self.selected.remove(&path);
                self.hidden.insert(path);
                self.persist(SETTING_SELECTION, &self.selected, shared);
                self.persist(SETTING_HIDDEN, &self.hidden, shared);
            }
            Action::Unhide(path) => {
                self.hidden.remove(&path);
                self.persist(SETTING_HIDDEN, &self.hidden, shared);
            }
        }
    }

    /// Zeichnet Apps als dreispaltiges Raster.
    fn grid(&self, ui: &mut Ui, p: &Palette, apps: &[&AppEntry], action: &mut Option<Action>) {
        ui.columns(COLUMNS, |columns| {
            for (index, app) in apps.iter().enumerate() {
                let ui = &mut columns[index % COLUMNS];
                ui.spacing_mut().item_spacing.y = 2.0;
                let is_selected = self.selected.contains(&app.path);
                let is_hidden = self.hidden.contains(&app.path);

                let row = theme::clickable_row(ui, p, ROW_HEIGHT, false, |ui| {
                    ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                        let mut checked = is_selected;
                        if ui.checkbox(&mut checked, "").changed() {
                            *action = Some(Action::ToggleSelected(app.path.clone()));
                        }
                        theme::app_icon(ui, p, self.textures.get(&app.path), &app.name, ICON_SIDE);
                        ui.add(Label::new(app.name.as_str()).truncate());
                    });
                })
                .on_hover_text(app.name.as_str());

                if row.clicked() {
                    *action = Some(Action::Launch(app.path.clone()));
                }
                row.context_menu(|ui| {
                    if ui.button(t("Starten")).clicked() {
                        *action = Some(Action::Launch(app.path.clone()));
                        ui.close_menu();
                    }
                    let label = if is_selected {
                        "Aus Auswahl entfernen"
                    } else {
                        "Zur Auswahl hinzufügen"
                    };
                    if ui.button(label).clicked() {
                        *action = Some(Action::ToggleSelected(app.path.clone()));
                        ui.close_menu();
                    }
                    let favorite_label = if self.favorites.contains(&app.path) {
                        "Aus Favoriten entfernen"
                    } else {
                        "Zu Favoriten hinzufügen"
                    };
                    if ui.button(favorite_label).clicked() {
                        *action = Some(Action::ToggleFavorite(app.path.clone()));
                        ui.close_menu();
                    }
                    if self.custom.iter().any(|(_, path)| path == &app.path)
                        && ui.button(t("Eigenen Eintrag entfernen")).clicked()
                    {
                        *action = Some(Action::AskRemoveCustom(app.path.clone()));
                        ui.close_menu();
                    }
                    ui.separator();
                    if is_hidden {
                        if ui.button(t("Wieder einblenden")).clicked() {
                            *action = Some(Action::Unhide(app.path.clone()));
                            ui.close_menu();
                        }
                    } else if ui.button(t("Ausblenden")).clicked() {
                        *action = Some(Action::Hide(app.path.clone()));
                        ui.close_menu();
                    }
                });
            }
        });
    }

    pub fn ui(&mut self, ui: &mut Ui, shared: &mut Shared) {
        let p = shared.palette;
        if !self.settings_loaded {
            self.settings_loaded = true;
            self.load_settings(shared);
        }
        // Eigene Einträge und Favoriten aus den Einstellungen übernehmen; neue Einträge
        // lösen einen Scan aus, damit sie samt Icon in der Liste erscheinen.
        if self.custom != shared.settings.launcher_custom {
            self.custom = shared.settings.launcher_custom.clone();
            self.start_scan(ui.ctx());
        }
        self.favorites = shared.settings.launcher_favorites.iter().cloned().collect();
        self.receive_scan(ui);
        let scanning = self.scan.is_some();
        let show_system = shared.settings.show_system_apps;

        // Nur zählen, was es nach dem letzten Scan noch gibt.
        let selected_paths: Vec<String> = self
            .apps
            .iter()
            .filter(|app| self.selected.contains(&app.path) && !self.hidden.contains(&app.path))
            .map(|app| app.path.clone())
            .collect();
        let hidden_count = self
            .apps
            .iter()
            .filter(|app| self.hidden.contains(&app.path))
            .count();

        // Sicherheitsabfrage vor dem Entfernen eigener Einträge.
        let mut confirmed: Option<Action> = None;
        if let Some(path) = self.confirm_remove.clone() {
            let name = self
                .custom
                .iter()
                .find(|(_, custom)| custom == &path)
                .map_or_else(|| path.clone(), |(name, _)| name.clone());
            theme::card(&p).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(crate::i18n::td(format!(
                        "Eintrag „{name}“ wirklich entfernen?"
                    )));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button(t("Abbrechen")).clicked() {
                            self.confirm_remove = None;
                        }
                        if ui
                            .button(RichText::new(t("Entfernen")).color(p.danger))
                            .clicked()
                        {
                            confirmed = Some(Action::RemoveCustom(path.clone()));
                        }
                    });
                });
            });
        }
        if let Some(action) = confirmed {
            self.apply(action, shared);
        }

        // ── Zeile 1: Suche + Start ──
        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if theme::primary_button(
                    ui,
                    &p,
                    t("Ausgewählte starten"),
                    !selected_paths.is_empty(),
                )
                .clicked()
                {
                    self.launch(&selected_paths, shared);
                }
                ui.add(
                    TextEdit::singleline(&mut self.search)
                        .hint_text(t("Apps durchsuchen"))
                        .desired_width(f32::INFINITY),
                );
            });
        });

        // ── Zeile 2: Filter ──
        let mut toggle_system = false;
        let mut rescan = false;
        ui.horizontal_wrapped(|ui| {
            if theme::chip(ui, &p, t("Alle"), self.filter == Filter::All).clicked() {
                self.filter = Filter::All;
            }
            let label = crate::i18n::td(format!("Auswahl ({})", selected_paths.len()));
            if theme::chip(ui, &p, &label, self.filter == Filter::Selected).clicked() {
                self.filter = Filter::Selected;
            }
            if hidden_count > 0 || self.filter == Filter::Hidden {
                let label = crate::i18n::td(format!("Ausgeblendet ({hidden_count})"));
                if theme::chip(ui, &p, &label, self.filter == Filter::Hidden).clicked() {
                    self.filter = Filter::Hidden;
                }
            }
            ui.separator();
            if theme::chip(ui, &p, t("Systemwerkzeuge"), show_system)
                .on_hover_text(t(
                    "Verwaltungsprogramme, Hilfedateien, Deinstallationsprogramme u. Ä. anzeigen",
                ))
                .clicked()
            {
                toggle_system = true;
            }
            if ui
                .add_enabled(!scanning, Button::new(t("Neu scannen")))
                .clicked()
            {
                rescan = true;
            }
        });
        if toggle_system {
            shared.settings.show_system_apps = !show_system;
            shared.save_settings();
        }
        if rescan {
            self.start_scan(ui.ctx());
        }

        // ── Lesezeichen: Sammlungen (mehrere Apps mit einem Klick) und eigene Einträge ──
        let mut launch_collection: Option<Vec<String>> = None;
        let mut load_collection: Option<Vec<String>> = None;
        let mut delete_collection: Option<usize> = None;
        let mut save_collection = false;
        let mut add_custom = false;
        theme::card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(t("Sammlungen")).strong());
                if shared.settings.launcher_collections.is_empty() {
                    ui.label(
                        theme::secondary(&p, t("Noch keine – wähle Apps per Häkchen aus und speichere sie hier."))
                            .size(12.5),
                    );
                }
                for (index, (name, paths)) in shared.settings.launcher_collections.iter().enumerate() {
                    let chip = theme::chip(ui, &p, &format!("▶ {name} ({})", paths.len()), false)
                        .on_hover_text(t("Klick startet alle Apps dieser Sammlung · Rechtsklick für mehr"));
                    if chip.clicked() {
                        launch_collection = Some(paths.clone());
                    }
                    chip.context_menu(|ui| {
                        if ui.button(t("Alle starten")).clicked() {
                            launch_collection = Some(paths.clone());
                            ui.close_menu();
                        }
                        if ui.button(t("Als Auswahl laden")).clicked() {
                            load_collection = Some(paths.clone());
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button(RichText::new(t("Sammlung löschen")).color(p.danger)).clicked() {
                            delete_collection = Some(index);
                            ui.close_menu();
                        }
                    });
                }
            });
            ui.horizontal(|ui| {
                ui.add(
                    TextEdit::singleline(&mut self.collection_name)
                        .hint_text(t("Name der neuen Sammlung"))
                        .desired_width(220.0),
                );
                let can_save = !selected_paths.is_empty() && !self.collection_name.trim().is_empty();
                let label = crate::i18n::td(format!("Auswahl speichern ({})", selected_paths.len()));
                if ui.add_enabled(can_save, Button::new(label)).clicked() {
                    save_collection = true;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .button(t("Eigene App hinzufügen …"))
                        .on_hover_text(t("Programme, Skripte (.bat, .cmd, .ps1), Verknüpfungen oder andere Dateien"))
                        .clicked()
                    {
                        add_custom = true;
                    }
                });
            });
        });
        if let Some(paths) = launch_collection {
            self.launch(&paths, shared);
        }
        if let Some(paths) = load_collection {
            self.selected = paths.into_iter().collect();
            self.persist(SETTING_SELECTION, &self.selected, shared);
        }
        if let Some(index) = delete_collection {
            shared.settings.launcher_collections.remove(index);
            shared.save_settings();
        }
        if save_collection {
            let name = self.collection_name.trim().to_owned();
            shared
                .settings
                .launcher_collections
                .push((name, selected_paths.clone()));
            self.collection_name.clear();
            shared.save_settings();
        }
        if add_custom {
            if let Some(files) = rfd::FileDialog::new()
                .set_title(t("Eigene App hinzufügen"))
                .pick_files()
            {
                for file in files {
                    let path = file.to_string_lossy().into_owned();
                    let name = file
                        .file_stem()
                        .map_or_else(|| path.clone(), |stem| stem.to_string_lossy().into_owned());
                    let custom = &mut shared.settings.launcher_custom;
                    if !custom.iter().any(|(_, existing)| existing == &path) {
                        custom.push((name, path));
                    }
                }
                shared.save_settings();
            }
        }

        let needle = self.search.trim().to_lowercase();
        let visible: Vec<&AppEntry> = self
            .apps
            .iter()
            .filter(|app| needle.is_empty() || app.name.to_lowercase().contains(&needle))
            .filter(|app| {
                let hidden = self.hidden.contains(&app.path);
                match self.filter {
                    Filter::Hidden => hidden,
                    Filter::Selected => !hidden && self.selected.contains(&app.path),
                    Filter::All => !hidden && (show_system || !app.system),
                }
            })
            .collect();

        if scanning && self.apps.is_empty() {
            theme::empty_state(ui, &p, t("Programme werden gesucht …"));
            return;
        }
        if visible.is_empty() {
            theme::empty_state(ui, &p, t("Keine Programme gefunden."));
            return;
        }
        ui.label(
            theme::secondary(
                &p,
                t("Klick startet die App · Häkchen für Mehrfachstart · Rechtsklick: Favorit, Ausblenden"),
            )
            .size(12.0),
        );

        let mut action = None;
        let grouped = needle.is_empty() && self.filter == Filter::All;
        if grouped {
            // Wie im Startmenü: Ordner mit mehreren Apps als aufklappbare Gruppe,
            // einzelne Apps (auch aus Ein-App-Ordnern) direkt in der Liste.
            let mut folders: BTreeMap<&str, Vec<&AppEntry>> = BTreeMap::new();
            for &app in &visible {
                folders.entry(app.folder.as_str()).or_default().push(app);
            }
            let mut loose: Vec<&AppEntry> = Vec::new();
            let mut groups: Vec<(&str, Vec<&AppEntry>)> = Vec::new();
            for (folder, apps) in folders {
                if folder.is_empty() || apps.len() == 1 {
                    loose.extend(apps);
                } else {
                    groups.push((folder, apps));
                }
            }
            loose.sort_by_key(|app| app.name.to_lowercase());

            // Favoriten stehen immer ganz oben.
            let favorites: Vec<&AppEntry> = visible
                .iter()
                .copied()
                .filter(|app| self.favorites.contains(&app.path))
                .collect();
            if !favorites.is_empty() {
                ui.label(RichText::new(t("Favoriten")).strong());
                theme::card(&p).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.grid(ui, &p, &favorites, &mut action);
                });
            }

            if !loose.is_empty() {
                theme::card(&p).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    self.grid(ui, &p, &loose, &mut action);
                });
            }
            for (folder, apps) in groups {
                let title = RichText::new(format!("{folder}  ({})", apps.len())).strong();
                CollapsingHeader::new(title)
                    .id_salt(("launcher-folder", folder))
                    .default_open(false)
                    .show(ui, |ui| {
                        self.grid(ui, &p, &apps, &mut action);
                    });
            }
        } else {
            theme::card(&p).show(ui, |ui| {
                ui.set_width(ui.available_width());
                self.grid(ui, &p, &visible, &mut action);
            });
        }

        if let Some(action) = action {
            self.apply(action, shared);
        }
    }
}
