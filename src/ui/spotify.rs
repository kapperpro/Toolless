use crate::i18n::t;
use eframe::egui::{self, Align2, Button, FontId, RichText, Sense, TextureHandle, Ui};

use crate::{
    app::Shared,
    services::spotify::{self, Command, Snapshot},
    theme::{self, Palette},
};

const ARTWORK_SIDE: f32 = 220.0;
const COLUMN_WIDTH: f32 = 380.0;
/// Mindestabstand zwischen zwei Lautstärke-Befehlen beim Ziehen (Sekunden).
const VOLUME_SEND_INTERVAL: f64 = 0.06;

#[derive(Default)]
pub struct SpotifyView {
    /// Zielposition (0.0 – 1.0), während am Fortschrittsbalken gezogen wird.
    seek_draft: Option<f32>,
    /// Cover als Textur samt der ID, zu der sie gehört.
    artwork: Option<(u64, TextureHandle)>,
    /// Lautstärke während des Ziehens – verhindert Springen durch das Polling.
    volume_draft: Option<f32>,
    last_volume_sent: f64,
}

fn format_time(ms: i64) -> String {
    let total = (ms.max(0) / 1000) as u64;
    format!("{}:{:02}", total / 60, total % 60)
}

#[derive(Clone, Copy)]
enum MediaIcon {
    Previous,
    Play,
    Pause,
    Next,
}

/// Zeichnet ein Wiedergabesymbol mittig in `rect`.
fn media_icon(ui: &Ui, rect: egui::Rect, icon: MediaIcon, color: egui::Color32) {
    let center = rect.center();
    let unit = rect.height() * 0.17; // halbe Symbolhöhe
    let painter = ui.painter();
    let triangle = |tip_x: f32, base_x: f32| {
        egui::Shape::convex_polygon(
            vec![
                egui::pos2(base_x, center.y - unit),
                egui::pos2(tip_x, center.y),
                egui::pos2(base_x, center.y + unit),
            ],
            color,
            egui::Stroke::NONE,
        )
    };
    let bar = |x: f32, width: f32| {
        egui::Rect::from_min_max(
            egui::pos2(x, center.y - unit),
            egui::pos2(x + width, center.y + unit),
        )
    };
    match icon {
        MediaIcon::Play => {
            // Leicht nach rechts versetzt wirkt das Dreieck optisch mittig.
            painter.add(triangle(center.x + unit * 1.05, center.x - unit * 0.75));
        }
        MediaIcon::Pause => {
            painter.rect_filled(bar(center.x - unit * 0.95, unit * 0.65), 1.0, color);
            painter.rect_filled(bar(center.x + unit * 0.30, unit * 0.65), 1.0, color);
        }
        MediaIcon::Next => {
            painter.add(triangle(center.x + unit * 0.55, center.x - unit * 1.0));
            painter.rect_filled(bar(center.x + unit * 0.65, unit * 0.4), 0.5, color);
        }
        MediaIcon::Previous => {
            painter.add(triangle(center.x - unit * 0.55, center.x + unit * 1.0));
            painter.rect_filled(bar(center.x - unit * 1.05, unit * 0.4), 0.5, color);
        }
    }
}

impl SpotifyView {
    /// Hält die Cover-Textur passend zum Schnappschuss.
    fn sync_artwork(&mut self, ui: &Ui, snapshot: &Snapshot) {
        match &snapshot.artwork {
            None => self.artwork = None,
            Some(artwork) => {
                if self.artwork.as_ref().map(|(id, _)| *id) != Some(artwork.id) {
                    let image = egui::ColorImage::from_rgba_unmultiplied(
                        [artwork.width, artwork.height],
                        &artwork.rgba,
                    );
                    let texture =
                        ui.ctx()
                            .load_texture("album-cover", image, egui::TextureOptions::LINEAR);
                    self.artwork = Some((artwork.id, texture));
                }
            }
        }
    }

    fn cover(&self, ui: &mut Ui, p: &Palette) {
        let size = egui::vec2(ARTWORK_SIDE, ARTWORK_SIDE);
        match &self.artwork {
            Some((_, texture)) => {
                let source = egui::load::SizedTexture::new(texture.id(), size);
                ui.add(egui::Image::new(source).rounding(p.card_radius));
            }
            None => {
                let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                ui.painter().rect_filled(rect, p.card_radius, p.card_bg);
                ui.painter().text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    "♪",
                    FontId::proportional(64.0),
                    p.text_secondary,
                );
            }
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, shared: &mut Shared) {
        if let Some(error) = shared.spotify.take_error() {
            shared.notify(error);
        }
        let p = shared.palette;
        let snapshot = shared.spotify.snapshot();
        self.sync_artwork(ui, &snapshot);
        let playback = snapshot.playback.as_ref();
        let is_playing = playback.is_some_and(|pb| pb.is_playing);

        // Fortschritt zwischen zwei Abfragen selbst weiterzählen → flüssiger Balken.
        let (mut progress_ms, duration_ms) =
            playback.map_or((0, 0), |pb| (pb.progress_ms, pb.duration_ms));
        if is_playing {
            if let Some(captured_at) = snapshot.captured_at {
                progress_ms += captured_at.elapsed().as_millis() as i64;
            }
            ui.ctx().request_repaint();
        }
        if duration_ms > 0 {
            progress_ms = progress_ms.min(duration_ms);
        }

        let mut open_error = None;
        // Alles in einer mittigen Spalte.
        let column = COLUMN_WIDTH.min(ui.available_width());
        let padding =
            ((ui.available_width() - column) / 2.0 - ui.spacing().item_spacing.x).max(0.0);
        ui.horizontal_top(|ui| {
            ui.add_space(padding);
            ui.vertical(|ui| {
                ui.set_width(column);
                ui.vertical_centered(|ui| {
                    ui.add_space(8.0);
                    self.cover(ui, &p);
                    ui.add_space(12.0);

                    let title = playback
                        .map(|pb| pb.title.as_str())
                        .filter(|t| !t.is_empty());
                    ui.label(
                        RichText::new(title.unwrap_or(t("Keine Wiedergabe")))
                            .size(20.0)
                            .strong(),
                    );
                    match playback {
                        Some(pb) => {
                            ui.label(theme::secondary(&p, pb.artist.clone()));
                            if !pb.album.is_empty() {
                                ui.label(theme::secondary(&p, pb.album.clone()).size(12.0));
                            }
                        }
                        None => {
                            ui.label(theme::secondary(
                                &p,
                                t("Starte einen Titel in Spotify – er erscheint hier automatisch."),
                            ));
                            ui.add_space(4.0);
                            if ui.button(t("Spotify öffnen")).clicked() {
                                open_error = spotify::open_spotify().err();
                            }
                        }
                    }
                    ui.add_space(10.0);

                    // Fortschrittsbalken: Klicken oder Ziehen spult; Zeiten an beiden Enden.
                    let (area, bar_response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 28.0),
                        Sense::click_and_drag(),
                    );
                    let bar = egui::Rect::from_min_size(
                        egui::pos2(area.left(), area.top() + 5.0),
                        egui::vec2(area.width(), 4.0),
                    );
                    let can_seek = playback.is_some() && duration_ms > 0;
                    if can_seek {
                        if let Some(pointer) = bar_response.interact_pointer_pos() {
                            let target = ((pointer.x - bar.left()) / bar.width()).clamp(0.0, 1.0);
                            self.seek_draft = Some(target);
                        }
                        if bar_response.drag_stopped() || bar_response.clicked() {
                            if let Some(target) = self.seek_draft.take() {
                                let position = (target as f64 * duration_ms as f64) as i64;
                                shared.spotify.send(Command::Seek(position));
                            }
                        }
                    }
                    if !bar_response.dragged() && !bar_response.is_pointer_button_down_on() {
                        self.seek_draft = None;
                    }
                    let fraction = match self.seek_draft {
                        Some(target) => target,
                        None if duration_ms > 0 => {
                            (progress_ms as f32 / duration_ms as f32).clamp(0.0, 1.0)
                        }
                        None => 0.0,
                    };
                    let shown_ms = match self.seek_draft {
                        Some(target) => (target as f64 * duration_ms as f64) as i64,
                        None => progress_ms,
                    };
                    ui.painter().rect_filled(bar, 2.0, p.control_hover);
                    let mut filled = bar;
                    filled.set_width(bar.width() * fraction);
                    ui.painter().rect_filled(filled, 2.0, p.accent);
                    if can_seek && (bar_response.hovered() || bar_response.dragged()) {
                        // Griff nur zeigen, wenn die Maus am Balken ist.
                        let knob = egui::pos2(filled.right(), bar.center().y);
                        ui.painter().circle_filled(knob, 6.0, p.accent);
                    }
                    let time_font = FontId::proportional(12.0);
                    ui.painter().text(
                        egui::pos2(area.left(), area.bottom()),
                        Align2::LEFT_BOTTOM,
                        format_time(shown_ms),
                        time_font.clone(),
                        p.text_secondary,
                    );
                    ui.painter().text(
                        egui::pos2(area.right(), area.bottom()),
                        Align2::RIGHT_BOTTOM,
                        format_time(duration_ms),
                        time_font,
                        p.text_secondary,
                    );
                    ui.add_space(2.0);

                    // Steuerung: zurück · Play/Pause · weiter (mittig). Die Symbole werden
                    // gezeichnet statt als Schriftzeichen gesetzt – so sitzen sie exakt mittig.
                    let (side, main, gap) = (40.0, 52.0, 14.0);
                    // Alle drei Knöpfe sitzen in einer fest berechneten Zeile auf derselben
                    // Mittellinie – unabhängig davon, wie egui die Zeilenhöhe bestimmt.
                    let total = 2.0 * side + main + 2.0 * gap;
                    let (row, _) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), main),
                        Sense::hover(),
                    );
                    let left = row.center().x - total / 2.0;
                    let slot = |x: f32, size: f32| {
                        egui::Rect::from_center_size(
                            egui::pos2(x + size / 2.0, row.center().y),
                            egui::vec2(size, size),
                        )
                    };

                    let previous = ui
                        .put(slot(left, side), Button::new("").rounding(side / 2.0))
                        .on_hover_text(t("Vorheriger Titel"));
                    media_icon(ui, previous.rect, MediaIcon::Previous, p.text);
                    if previous.clicked() {
                        shared.spotify.send(Command::Previous);
                    }

                    let play = ui
                        .put(
                            slot(left + side + gap, main),
                            Button::new("").fill(p.accent).rounding(main / 2.0),
                        )
                        .on_hover_text(if is_playing { "Pause" } else { "Wiedergabe" });
                    let icon = if is_playing {
                        MediaIcon::Pause
                    } else {
                        MediaIcon::Play
                    };
                    media_icon(ui, play.rect, icon, p.on_accent);
                    if play.clicked() {
                        shared.spotify.send(Command::Toggle);
                    }

                    let next = ui
                        .put(
                            slot(left + side + gap + main + gap, side),
                            Button::new("").rounding(side / 2.0),
                        )
                        .on_hover_text(t("Nächster Titel"));
                    media_icon(ui, next.rect, MediaIcon::Next, p.text);
                    if next.clicked() {
                        shared.spotify.send(Command::Next);
                    }

                    // Lautstärke der Spotify-App (Windows-Lautstärkemixer).
                    if shared.settings.show_volume {
                        if let Some(device_volume) = snapshot.volume {
                            ui.add_space(8.0);
                            let slider_width = 180.0;
                            ui.horizontal(|ui| {
                                let total = slider_width + 110.0;
                                ui.add_space(((ui.available_width() - total) / 2.0).max(0.0));
                                ui.label(theme::secondary(&p, t("Lautstärke")).size(12.0));

                                let mut value = self.volume_draft.unwrap_or(device_volume * 100.0);
                                let slider =
                                    theme::slider(ui, &p, &mut value, 0.0, 100.0, slider_width);
                                ui.label(
                                    theme::secondary(&p, format!("{} %", value.round() as u32))
                                        .size(12.0),
                                );

                                if slider.changed() {
                                    self.volume_draft = Some(value);
                                    // Beim Ziehen direkt hörbar, aber nicht öfter als nötig senden.
                                    if shared.now - self.last_volume_sent >= VOLUME_SEND_INTERVAL {
                                        self.last_volume_sent = shared.now;
                                        shared.spotify.send(Command::Volume(value / 100.0));
                                    }
                                }
                                if slider.drag_stopped() {
                                    shared.spotify.send(Command::Volume(value / 100.0));
                                }
                                // Entwurf verwerfen, sobald Spotify den neuen Wert meldet.
                                let reported = (device_volume * 100.0).round();
                                if !slider.dragged()
                                    && self.volume_draft.map(f32::round) == Some(reported)
                                {
                                    self.volume_draft = None;
                                }
                            });
                        }
                    }
                });
            });
        });

        if let Some(err) = open_error {
            shared.notify(err);
        }
    }
}
