use eframe::egui::{self, Align, Button, Color32, Layout, RichText, Sense, Slider, TextEdit, Ui};

use crate::{
    app::Shared,
    services::spotify::{Command, Snapshot, REDIRECT_URI},
    theme,
};

#[derive(Default)]
pub struct SpotifyView {
    client_id: String,
    client_id_prefilled: bool,
    /// Lautstärke während des Ziehens – verhindert Springen durch das Polling.
    volume_draft: Option<f32>,
}

fn format_time(ms: i64) -> String {
    let total = (ms.max(0) / 1000) as u64;
    format!("{}:{:02}", total / 60, total % 60)
}

impl SpotifyView {
    pub fn ui(&mut self, ui: &mut Ui, shared: &mut Shared) {
        if let Some(error) = shared.spotify.take_error() {
            shared.notify(error);
        }
        let snapshot = shared.spotify.snapshot();
        ui.set_max_width(560.0);

        if snapshot.authenticated {
            self.player(ui, shared, &snapshot);
        } else {
            self.setup(ui, shared, &snapshot);
        }
    }

    /// Einrichtung: Client-ID eintragen und per Browser anmelden.
    fn setup(&mut self, ui: &mut Ui, shared: &mut Shared, snapshot: &Snapshot) {
        let p = shared.palette;
        if !self.client_id_prefilled && !snapshot.client_id.is_empty() {
            self.client_id = snapshot.client_id.clone();
            self.client_id_prefilled = true;
        }

        theme::card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::card_title(ui, "Mit Spotify verbinden");
            ui.label(theme::secondary(
                &p,
                "1. Lege im Spotify Developer Dashboard (developer.spotify.com/dashboard) eine App an und wähle „Web API“.",
            ));
            ui.horizontal_wrapped(|ui| {
                ui.label(theme::secondary(&p, "2. Trage dort als Redirect URI exakt ein:"));
                ui.label(RichText::new(REDIRECT_URI).monospace());
                if ui.small_button("Kopieren").clicked() {
                    ui.ctx().copy_text(REDIRECT_URI.to_owned());
                }
            });
            ui.label(theme::secondary(
                &p,
                "3. Kopiere die Client-ID hierher und verbinde dich. Ein Client-Secret wird nicht benötigt.",
            ));
            ui.add_space(4.0);

            ui.horizontal(|ui| {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let label = if snapshot.connecting { "Warte auf Browser …" } else { "Verbinden" };
                    let enabled = !snapshot.connecting && !self.client_id.trim().is_empty();
                    if theme::primary_button(ui, &p, label, enabled).clicked() {
                        shared.spotify.send(Command::Login(self.client_id.clone()));
                    }
                    ui.add(
                        TextEdit::singleline(&mut self.client_id)
                            .hint_text("Client-ID")
                            .desired_width(f32::INFINITY),
                    );
                });
            });
            ui.add_space(4.0);
            ui.label(
                theme::secondary(
                    &p,
                    "Die Wiedergabesteuerung über die Web API setzt Spotify Premium und ein aktives Gerät voraus.",
                )
                .size(12.0),
            );
        });
    }

    fn player(&mut self, ui: &mut Ui, shared: &mut Shared, snapshot: &Snapshot) {
        let p = shared.palette;
        let playback = snapshot.playback.as_ref();

        theme::card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add_space(8.0);
            ui.vertical_centered(|ui| {
                let title = playback
                    .map(|pb| pb.title.as_str())
                    .filter(|t| !t.is_empty());
                ui.label(
                    RichText::new(title.unwrap_or("Keine Wiedergabe"))
                        .size(20.0)
                        .strong(),
                );
                let subtitle = match playback {
                    Some(pb) => pb.artist.clone(),
                    None => "Starte die Wiedergabe in Spotify, um sie hier zu steuern.".to_owned(),
                };
                ui.label(theme::secondary(&p, subtitle));
            });
            ui.add_space(10.0);

            // Fortschrittsbalken
            let (progress_ms, duration_ms) =
                playback.map_or((0, 0), |pb| (pb.progress_ms, pb.duration_ms));
            let fraction = if duration_ms > 0 {
                (progress_ms as f32 / duration_ms as f32).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (bar, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 4.0), Sense::hover());
            ui.painter().rect_filled(bar, 2.0, p.control_hover);
            let mut filled = bar;
            filled.set_width(bar.width() * fraction);
            ui.painter().rect_filled(filled, 2.0, p.text_secondary);
            ui.horizontal(|ui| {
                ui.label(theme::secondary(&p, format_time(progress_ms)).size(12.0));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(theme::secondary(&p, format_time(duration_ms)).size(12.0));
                });
            });

            // Steuerung: zurück · Play/Pause · weiter (mittig)
            let (side, main, gap) = (44.0, 56.0, 16.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                let total = 2.0 * side + main + 2.0 * gap;
                ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));

                let small = |symbol: &str| {
                    Button::new(RichText::new(symbol).size(20.0)).rounding(side / 2.0)
                };
                if ui
                    .add_sized([side, side], small("⏮"))
                    .on_hover_text("Vorheriger Titel")
                    .clicked()
                {
                    shared.spotify.send(Command::Previous);
                }
                let is_playing = playback.is_some_and(|pb| pb.is_playing);
                let symbol = if is_playing { "⏸" } else { "▶" };
                let (fill, color) = if p.dark {
                    (Color32::from_gray(235), Color32::from_gray(20))
                } else {
                    (Color32::from_gray(28), Color32::WHITE)
                };
                let play = Button::new(RichText::new(symbol).size(24.0).color(color))
                    .fill(fill)
                    .rounding(main / 2.0);
                if ui
                    .add_sized([main, main], play)
                    .on_hover_text(if is_playing { "Pause" } else { "Wiedergabe" })
                    .clicked()
                {
                    shared.spotify.send(Command::Toggle);
                }
                if ui
                    .add_sized([side, side], small("⏭"))
                    .on_hover_text("Nächster Titel")
                    .clicked()
                {
                    shared.spotify.send(Command::Next);
                }
            });

            // Lautstärke
            if let Some(device_volume) = playback.and_then(|pb| pb.volume) {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(theme::secondary(&p, "Lautstärke").size(12.0));
                    let mut value = self.volume_draft.unwrap_or(device_volume as f32);
                    ui.spacing_mut().slider_width = (ui.available_width() - 60.0).max(80.0);
                    let slider = ui.add(Slider::new(&mut value, 0.0..=100.0).show_value(false));
                    ui.label(
                        theme::secondary(&p, format!("{} %", value.round() as u32)).size(12.0),
                    );

                    if slider.changed() {
                        self.volume_draft = Some(value);
                    }
                    // Erst beim Loslassen senden (bzw. sofort bei Tastaturbedienung).
                    if slider.drag_stopped() || (slider.changed() && !slider.dragged()) {
                        shared.spotify.send(Command::Volume(value.round() as u8));
                    }
                    // Entwurf verwerfen, sobald Spotify den neuen Wert meldet.
                    if !slider.dragged()
                        && self.volume_draft.map(|v| v.round() as u32) == Some(device_volume)
                    {
                        self.volume_draft = None;
                    }
                });
            }
            ui.add_space(4.0);
        });

        theme::card(&p).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    let device = playback
                        .map(|pb| pb.device.as_str())
                        .filter(|d| !d.is_empty());
                    ui.label(RichText::new(device.unwrap_or("Kein aktives Gerät")).strong());
                    ui.label(theme::secondary(&p, "Wiedergabegerät").size(12.0));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Trennen").clicked() {
                        shared.spotify.send(Command::Logout);
                    }
                    if ui.button("Aktualisieren").clicked() {
                        shared.spotify.send(Command::Refresh);
                    }
                });
            });
        });
    }
}
