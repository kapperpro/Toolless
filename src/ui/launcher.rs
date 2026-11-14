use std::{
    collections::BTreeSet,
    sync::mpsc::{self, Receiver},
    thread,
};

use eframe::egui::{Align, Button, Layout, TextEdit, Ui};

use crate::{
    app::Shared,
    services::launcher::{self, AppEntry},
    theme,
};

const SETTING_SELECTION: &str = "launcher_selection";
const COLUMNS: usize = 3;

#[derive(Default)]
pub struct LauncherView {
    started: bool,
    apps: Vec<AppEntry>,
    /// Läuft gerade ein Scan, liefert dieser Kanal das Ergebnis.
    scan: Option<Receiver<Vec<AppEntry>>>,
    search: String,
    only_selected: bool,
    /// Auswahl (Pfade) – bleibt gespeichert, um dasselbe App-Set erneut zu starten.
    selected: BTreeSet<String>,
}

impl LauncherView {
    fn start_scan(&mut self, ui: &Ui) {
        let (tx, rx) = mpsc::channel();
        let ctx = ui.ctx().clone();
        thread::spawn(move || {
            let _ = tx.send(launcher::scan());
            ctx.request_repaint();
        });
        self.scan = Some(rx);
    }

    fn persist_selection(&self, shared: &mut Shared) {
        let json = serde_json::to_string(&self.selected).unwrap_or_else(|_| "[]".into());
        if let Err(err) = shared.db.set_setting(SETTING_SELECTION, &json) {
            shared.notify(err);
        }
    }

    fn launch(&self, paths: &[String], shared: &mut Shared) {
        let (started, error) = launcher::launch(paths);
        match error {
            None => shared.notify(format!(
                "{started} {} gestartet",
                if started == 1 { "App" } else { "Apps" }
            )),
            Some(error) => shared.notify(format!(
                "{} von {} fehlgeschlagen: {error}",
                paths.len() - started,
                paths.len()
            )),
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, shared: &mut Shared) {
        let p = shared.palette;
        if !self.started {
            self.started = true;
            if let Ok(Some(json)) = shared.db.get_setting(SETTING_SELECTION) {
                self.selected = serde_json::from_str(&json).unwrap_or_default();
            }
            self.start_scan(ui);
        }
        if let Some(apps) = self.scan.as_ref().and_then(|rx| rx.try_recv().ok()) {
            self.apps = apps;
            self.scan = None;
        }
        let scanning = self.scan.is_some();

        // Nur zählen, was es nach dem letzten Scan noch gibt.
        let selected_paths: Vec<String> = self
            .apps
            .iter()
            .filter(|app| self.selected.contains(&app.path))
            .map(|app| app.path.clone())
            .collect();

        ui.horizontal(|ui| {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if theme::primary_button(ui, &p, "Ausgewählte starten", !selected_paths.is_empty())
                    .clicked()
                {
                    self.launch(&selected_paths, shared);
                }
                if ui
                    .add_enabled(!scanning, Button::new("Neu scannen"))
                    .clicked()
                {
                    self.start_scan(ui);
                }
                let label = format!("Auswahl ({})", selected_paths.len());
                if theme::chip(ui, &p, &label, self.only_selected).clicked() {
                    self.only_selected = !self.only_selected;
                }
                ui.add(
                    TextEdit::singleline(&mut self.search)
                        .hint_text("Apps durchsuchen")
                        .desired_width(f32::INFINITY),
                );
            });
        });

        let needle = self.search.trim().to_lowercase();
        let visible: Vec<&AppEntry> = self
            .apps
            .iter()
            .filter(|app| needle.is_empty() || app.name.to_lowercase().contains(&needle))
            .filter(|app| !self.only_selected || self.selected.contains(&app.path))
            .collect();

        if scanning && self.apps.is_empty() {
            theme::empty_state(ui, &p, "Programme werden gesucht …");
            return;
        }
        if visible.is_empty() {
            theme::empty_state(ui, &p, "Keine Programme gefunden.");
            return;
        }

        let mut toggled = None;
        let mut launch_one = None;
        theme::card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.columns(COLUMNS, |columns| {
                for (index, app) in visible.iter().enumerate() {
                    let ui = &mut columns[index % COLUMNS];
                    ui.horizontal(|ui| {
                        let mut checked = self.selected.contains(&app.path);
                        if ui.checkbox(&mut checked, "").changed() {
                            toggled = Some(app.path.clone());
                        }
                        let entry = Button::new(app.name.as_str()).frame(false);
                        if ui.add(entry).on_hover_text("Klicken zum Starten").clicked() {
                            launch_one = Some(app.path.clone());
                        }
                    });
                }
            });
        });

        if let Some(path) = toggled {
            if !self.selected.remove(&path) {
                self.selected.insert(path);
            }
            self.persist_selection(shared);
        }
        if let Some(path) = launch_one {
            self.launch(&[path], shared);
        }
    }
}
