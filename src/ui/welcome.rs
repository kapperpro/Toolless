//! Willkommensfenster beim ersten Start: Sprache, Erscheinungsbild und eine kurze Einführung.

use eframe::egui::{self, Align, Align2, Color32, Id, Layout, Order, RichText, Sense};

use crate::{
    app::Shared,
    i18n::t,
    settings::{Accent, Appearance, Language},
    theme::{self, Palette},
    ui::system::preview,
};

/// Sprache, Erscheinungsbild und drei Seiten Einführung.
const STEPS: usize = 5;

#[derive(Default)]
pub struct Welcome {
    step: usize,
}

/// Überschrift und Absätze einer Einführungsseite.
fn tutorial(step: usize) -> (&'static str, [&'static str; 3]) {
    match step {
        2 => (
            "Die Seitenleiste",
            [
                "Links wählst du das Werkzeug: Spotify, Zwischenablage, Notizen, Rechner, App-Starter, Task-Manager, Minecraft, Spielzeit und Uhr.",
                "Einträge lassen sich per Ziehen umsortieren; ein Rechtsklick blendet sie aus.",
                "Mit dem Symbol ganz oben klappt die Leiste auf reine Symbole zusammen.",
            ],
        ),
        3 => (
            "Einstellungen",
            [
                "Das Zahnrad oben rechts neben „Minimieren“ öffnet die Einstellungen als großes Fenster – jederzeit.",
                "Dort änderst du Erscheinungsbild, Akzentfarbe, Größe und kompakte Darstellung, sortierst die Werkzeuge und sicherst alle Daten in einer Datei.",
                "Auf Wunsch erscheinen die Einstellungen auch als Eintrag in der Seitenleiste.",
            ],
        ),
        _ => (
            "Gut zu wissen",
            [
                "Die Uhr bietet Wecker, mehrere Countdowns und Stoppuhren sowie AFK-, Uhr- und Spotify-Vollbild. Esc, 20 Sekunden gehalten, beendet sie immer.",
                "Der Task-Manager zeichnet im Hintergrund auf – auch wenn das Fenster im Tray versteckt ist.",
                "Alle Daten bleiben auf diesem Rechner und sind mit deinem Windows-Konto verschlüsselt.",
            ],
        ),
    }
}

impl Welcome {
    /// Zeichnet das Fenster; liefert `true`, sobald es geschlossen werden soll.
    pub fn show(&mut self, ctx: &egui::Context, shared: &mut Shared) -> bool {
        let p = shared.palette;
        let screen = ctx.screen_rect();
        egui::Area::new(Id::new("welcome_dim"))
            .order(Order::Foreground)
            .fixed_pos(screen.min)
            .show(ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(screen.size(), Sense::click());
                ui.painter()
                    .rect_filled(rect, 0.0, Color32::from_black_alpha(200));
            });

        let mut changed = false;
        let mut finished = false;
        let step = self.step;
        let mut next_step = step;
        let system_dark = ctx.theme() == egui::Theme::Dark;
        egui::Area::new(Id::new("welcome_window"))
            .order(Order::Foreground)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                egui::Frame::none()
                    .fill(p.content_bg)
                    .stroke(egui::Stroke::new(1.0_f32, p.separator))
                    .rounding(p.radius + 4.0)
                    .inner_margin(egui::Margin::symmetric(26.0, 20.0))
                    .show(ui, |ui| {
                        ui.set_width(560.0);
                        ui.label(
                            theme::secondary(
                                &p,
                                format!("{} / {STEPS}", step + 1),
                            )
                            .size(12.0),
                        );
                        match step {
                            0 => {
                                ui.label(RichText::new("Toolless").size(28.0).strong());
                                ui.add_space(4.0);
                                ui.label(t("Willkommen! Wähle zuerst deine Sprache."));
                                ui.add_space(10.0);
                                ui.horizontal(|ui| {
                                    for option in Language::ALL {
                                        let active = shared.settings.language == option;
                                        if theme::chip(ui, &p, option.label(), active).clicked() {
                                            shared.settings.language = option;
                                            changed = true;
                                        }
                                    }
                                });
                            }
                            1 => {
                                ui.label(RichText::new(t("Erscheinungsbild")).size(22.0).strong());
                                ui.add_space(2.0);
                                ui.label(
                                    theme::secondary(
                                        &p,
                                        t("Wähle ein Design und eine Akzentfarbe – beides lässt sich später in den Einstellungen ändern."),
                                    )
                                    .size(12.5),
                                );
                                ui.add_space(8.0);
                                ui.horizontal_wrapped(|ui| {
                                    for option in Appearance::ALL {
                                        let mut sample = shared.settings.clone();
                                        sample.appearance = option;
                                        let palette = Palette::from_settings(system_dark, &sample);
                                        ui.vertical(|ui| {
                                            let current = shared.settings.appearance == option;
                                            if preview(
                                                ui,
                                                egui::vec2(120.0, 70.0),
                                                &palette,
                                                current.then_some(p.accent),
                                            )
                                            .clicked()
                                            {
                                                shared.settings.appearance = option;
                                                changed = true;
                                            }
                                            ui.label(theme::secondary(&p, t(option.label())).size(12.0));
                                        });
                                    }
                                });
                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    for accent in Accent::ALL {
                                        let (rect, response) = ui
                                            .allocate_exact_size(egui::vec2(24.0, 24.0), Sense::click());
                                        ui.painter().circle_filled(
                                            rect.center(),
                                            9.0,
                                            accent.color(p.dark),
                                        );
                                        if shared.settings.custom_accent.is_none()
                                            && shared.settings.accent == accent
                                        {
                                            ui.painter().circle_stroke(
                                                rect.center(),
                                                11.5,
                                                egui::Stroke::new(2.0_f32, p.text),
                                            );
                                        }
                                        if response.on_hover_text(t(accent.label())).clicked() {
                                            shared.settings.accent = accent;
                                            shared.settings.custom_accent = None;
                                            changed = true;
                                        }
                                    }
                                });
                            }
                            _ => {
                                let (title, lines) = tutorial(step);
                                ui.label(RichText::new(t(title)).size(22.0).strong());
                                ui.add_space(6.0);
                                for line in lines {
                                    ui.label(t(line));
                                    ui.add_space(4.0);
                                }
                            }
                        }

                        ui.add_space(14.0);
                        ui.horizontal(|ui| {
                            if ui.button(t("Überspringen")).clicked() {
                                finished = true;
                            }
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if step + 1 < STEPS {
                                    if theme::primary_button(ui, &p, t("Weiter"), true).clicked() {
                                        next_step = step + 1;
                                    }
                                } else if theme::primary_button(ui, &p, t("Los geht’s"), true)
                                    .clicked()
                                {
                                    finished = true;
                                }
                                if step > 0 && ui.button(t("Zurück")).clicked() {
                                    next_step = step - 1;
                                }
                            });
                        });
                    });
            });

        self.step = next_step;
        if finished {
            shared.settings.onboarded = true;
            changed = true;
        }
        if changed {
            shared.save_settings();
        }
        finished
    }
}
