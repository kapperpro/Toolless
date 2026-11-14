use eframe::egui::{Align, Layout, RichText, Ui};

use crate::{
    app::Shared,
    services::autostart,
    theme::{self, Palette},
};

#[derive(Default)]
pub struct SystemView {
    /// Autostart-Zustand wird beim ersten Anzeigen einmal abgefragt.
    autostart: Option<bool>,
}

/// Eine Einstellungszeile: Titel + Beschreibung links, Schalter rechts.
fn setting_row(ui: &mut Ui, p: &Palette, title: &str, description: &str, value: &mut bool) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_max_width((ui.available_width() - 60.0).max(120.0));
            ui.label(RichText::new(title).strong());
            ui.label(theme::secondary(p, description).size(12.5));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            changed = theme::toggle(ui, p, value).changed();
        });
    });
    changed
}

impl SystemView {
    pub fn ui(&mut self, ui: &mut Ui, shared: &mut Shared) {
        let p = shared.palette;
        ui.set_max_width(580.0);

        theme::card(&p).show(ui, |ui| {
            let mut keep_awake = shared.power.is_active();
            if setting_row(
                ui,
                &p,
                "Ruhezustand blockieren",
                "Verhindert Standby und Bildschirm-Aus, solange der Schalter aktiv ist. Endet automatisch mit der App.",
                &mut keep_awake,
            ) {
                if let Err(err) = shared.power.set(keep_awake) {
                    shared.notify(err);
                }
            }

            ui.separator();

            let mut enabled = *self.autostart.get_or_insert_with(|| autostart::is_enabled().unwrap_or(false));
            if setting_row(
                ui,
                &p,
                "Mit dem System starten",
                "Trägt Toolless in den Autostart ein. Die App startet dann unauffällig im System-Tray.",
                &mut enabled,
            ) {
                match autostart::set(enabled) {
                    Ok(state) => self.autostart = Some(state),
                    Err(err) => shared.notify(err),
                }
            }
        });

        theme::card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Hintergrundbetrieb").strong());
            ui.label(
                theme::secondary(
                    &p,
                    "Schließen versteckt nur das Fenster. Über das Tray-Icon öffnest du Toolless wieder, \
                     steuerst Spotify oder beendest die App vollständig.",
                )
                .size(12.5),
            );
        });
    }
}
