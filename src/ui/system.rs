use eframe::egui::{self, Align, Color32, Layout, RichText, Sense, Stroke, Ui};

use crate::i18n::t;
use crate::{
    app::{self, Shared, Tab},
    platform,
    services::autostart,
    settings::{Accent, Appearance, Language, Settings, ZOOM_LEVELS},
    theme::{self, Palette},
};

#[derive(Default)]
pub struct SystemView {
    /// Autostart-Zustand wird beim ersten Anzeigen einmal abgefragt.
    autostart: Option<bool>,
}

/// Einstellungszeile: Titel (+ Beschreibung) links, Bedienelement rechts.
fn row(
    ui: &mut Ui,
    p: &Palette,
    title: &str,
    description: &str,
    add_control: impl FnOnce(&mut Ui),
) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_max_width((ui.available_width() * 0.5).max(160.0));
            ui.label(RichText::new(t(title)).strong());
            if !description.is_empty() {
                ui.label(theme::secondary(p, t(description)).size(12.5));
            }
        });
        ui.with_layout(Layout::right_to_left(Align::Center), add_control);
    });
}

/// Wie `row`, aber mit den Bedienelementen in einer eigenen Zeile – für breite
/// Steuerelemente, die neben der Beschreibung keinen Platz haben.
fn stacked_row(
    ui: &mut Ui,
    p: &Palette,
    title: &str,
    description: &str,
    add_control: impl FnOnce(&mut Ui),
) {
    ui.label(RichText::new(t(title)).strong());
    if !description.is_empty() {
        ui.label(theme::secondary(p, t(description)).size(12.5));
    }
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), add_control);
    });
}

fn section(ui: &mut Ui, p: &Palette, title: &str, add_contents: impl FnOnce(&mut Ui)) {
    ui.add_space(4.0);
    ui.label(RichText::new(t(title)).strong());
    theme::card(p).show(ui, |ui| {
        ui.set_width(ui.available_width());
        add_contents(ui);
    });
}

/// Farbwähler für eine optionale eigene Farbe. `current` ist die derzeit wirksame Farbe.
/// Liefert `true`, wenn sich die Einstellung geändert hat.
fn color_option(ui: &mut Ui, custom: &mut Option<[u8; 3]>, current: Color32) -> bool {
    let mut changed = false;
    // Von rechts nach links: zuerst „Standard“, links daneben das Farbfeld.
    if ui
        .add_enabled(custom.is_some(), egui::Button::new(t("Standard")))
        .on_hover_text(t("Eigene Farbe entfernen"))
        .clicked()
    {
        *custom = None;
        changed = true;
    }
    let mut color = custom.unwrap_or_else(|| theme::to_rgb(current));
    if ui.color_edit_button_srgb(&mut color).changed() {
        *custom = Some(color);
        changed = true;
    }
    changed
}

impl SystemView {
    fn design(&mut self, ui: &mut Ui, p: &Palette, settings: &mut Settings, changed: &mut bool) {
        row(ui, p, t("Sprache"), "Language", |ui| {
            for option in Language::ALL.into_iter().rev() {
                if theme::chip(ui, p, option.label(), settings.language == option).clicked() {
                    settings.language = option;
                    *changed = true;
                }
            }
        });
        ui.separator();

        row(
            ui,
            p,
            t("Erscheinungsbild"),
            t("Hell, dunkel oder wie das System."),
            |ui| {
                // Von rechts nach links angeordnet → in umgekehrter Reihenfolge einfügen.
                for option in Appearance::ALL.into_iter().rev() {
                    if theme::chip(ui, p, option.label(), settings.appearance == option).clicked() {
                        settings.appearance = option;
                        *changed = true;
                    }
                }
            },
        );
        ui.separator();

        stacked_row(
            ui,
            p,
            t("Akzentfarbe"),
            t("Vorgabe wählen oder rechts eine eigene Farbe mischen."),
            |ui| {
                *changed |= color_option(ui, &mut settings.custom_accent, p.accent);
                for accent in Accent::ALL.into_iter().rev() {
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(22.0, 22.0), Sense::click());
                    let response = response.on_hover_text(accent.label());
                    ui.painter()
                        .circle_filled(rect.center(), 8.0, accent.color(p.dark));
                    if settings.custom_accent.is_none() && settings.accent == accent {
                        ui.painter().circle_stroke(
                            rect.center(),
                            10.5,
                            Stroke::new(2.0_f32, p.text),
                        );
                    }
                    if response.clicked() {
                        settings.accent = accent;
                        settings.custom_accent = None;
                        *changed = true;
                    }
                }
            },
        );
        ui.separator();

        row(
            ui,
            p,
            t("Hintergrund"),
            t("Text und Flächen passen sich der Helligkeit automatisch an."),
            |ui| {
                *changed |= color_option(ui, &mut settings.custom_background, p.content_bg);
            },
        );
        ui.separator();
        row(
            ui,
            p,
            t("Seitenleiste"),
            t("Farbe der Navigation und der Titelleiste."),
            |ui| {
                *changed |= color_option(ui, &mut settings.custom_sidebar, p.sidebar_bg);
            },
        );
        ui.separator();
        row(
            ui,
            p,
            t("Karten"),
            t("Farbe der abgesetzten Flächen."),
            |ui| {
                *changed |= color_option(ui, &mut settings.custom_card, p.card_bg);
            },
        );
        ui.separator();

        row(
            ui,
            p,
            t("Ecken"),
            t("0 = eckig, höhere Werte = runder."),
            |ui| {
                let mut value = settings.rounding;
                let slider = theme::slider(ui, p, &mut value, 0.0, 12.0, 160.0);
                // Nur ganze Stufen.
                settings.rounding = value.round();
                ui.label(theme::secondary(p, format!("{:.0}", settings.rounding)));
                // Erst beim Loslassen speichern, damit nicht jeder Zwischenwert geschrieben wird.
                if slider.drag_stopped() || (slider.changed() && !slider.dragged()) {
                    *changed = true;
                }
            },
        );
        ui.separator();

        row(
            ui,
            p,
            t("Größe"),
            t("Skaliert die gesamte Oberfläche."),
            |ui| {
                for (zoom, label) in ZOOM_LEVELS.into_iter().rev() {
                    let active = (settings.zoom - zoom).abs() < 0.01;
                    if theme::chip(ui, p, label, active).clicked() {
                        settings.zoom = zoom;
                        *changed = true;
                    }
                }
            },
        );
        ui.separator();

        row(
            ui,
            p,
            t("Kompakte Darstellung"),
            t("Engere Abstände und kleinere Schaltflächen."),
            |ui| {
                *changed |= theme::toggle(ui, p, &mut settings.compact).changed();
            },
        );
        ui.separator();

        row(
            ui,
            p,
            t("Design zurücksetzen"),
            t("Stellt Farben, Ecken und Größe auf den Standard."),
            |ui| {
                if ui.button(t("Zurücksetzen")).clicked() {
                    let defaults = Settings::default();
                    settings.appearance = defaults.appearance;
                    settings.accent = defaults.accent;
                    settings.custom_accent = None;
                    settings.custom_background = None;
                    settings.custom_sidebar = None;
                    settings.custom_card = None;
                    settings.rounding = defaults.rounding;
                    settings.zoom = defaults.zoom;
                    settings.compact = defaults.compact;
                    *changed = true;
                }
            },
        );
    }

    /// Werkzeuge der Seitenleiste ein-/ausblenden und umsortieren.
    fn tools(&mut self, ui: &mut Ui, p: &Palette, settings: &mut Settings, changed: &mut bool) {
        let tabs = app::ordered_tabs(settings);
        let count = tabs.len();
        for (index, tab) in tabs.into_iter().enumerate() {
            let hidden = app::is_tab_hidden(settings, tab);
            ui.horizontal(|ui| {
                let label = RichText::new(tab.label());
                ui.label(if hidden {
                    label.color(p.text_secondary)
                } else {
                    label.strong()
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let mut visible = !hidden;
                    if tab.can_hide() {
                        if theme::toggle(ui, p, &mut visible).changed() {
                            app::set_tab_hidden(settings, tab, !visible);
                            *changed = true;
                        }
                    } else {
                        ui.label(theme::secondary(p, t("immer sichtbar")).size(12.0));
                    }
                    if ui
                        .add_enabled(index + 1 < count, egui::Button::new(t("Runter")))
                        .clicked()
                    {
                        app::move_tab(settings, tab, 1);
                        *changed = true;
                    }
                    if ui
                        .add_enabled(index > 0, egui::Button::new(t("Hoch")))
                        .clicked()
                    {
                        app::move_tab(settings, tab, -1);
                        *changed = true;
                    }
                });
            });
            if index + 1 < count {
                ui.separator();
            }
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, shared: &mut Shared) {
        let p = shared.palette;
        ui.set_max_width(680.0);
        let mut changed = false;
        let mut error = None;

        section(ui, &p, t("Design"), |ui| {
            self.design(ui, &p, &mut shared.settings, &mut changed);
        });

        section(ui, &p, t("Werkzeuge"), |ui| {
            ui.label(
                theme::secondary(
                    &p,
                    t("Reihenfolge und Sichtbarkeit in der Seitenleiste. Dort lassen sich die Einträge auch direkt ziehen oder per Rechtsklick anpassen."),
                )
                .size(12.5),
            );
            ui.separator();
            self.tools(ui, &p, &mut shared.settings, &mut changed);
        });

        section(ui, &p, t("Verhalten"), |ui| {
            let mut keep_awake = shared.power.is_active();
            row(
                ui,
                &p,
                t("Ruhezustand blockieren"),
                t("Verhindert Standby und Bildschirm-Aus, solange der Schalter aktiv ist."),
                |ui| {
                    if theme::toggle(ui, &p, &mut keep_awake).changed() {
                        error = shared.power.set(keep_awake).err();
                    }
                },
            );
            ui.separator();

            let mut enabled = *self
                .autostart
                .get_or_insert_with(|| autostart::is_enabled().unwrap_or(false));
            row(
                ui,
                &p,
                t("Mit dem System starten"),
                t("Trägt Toolless in den Autostart ein; die App startet dann im System-Tray."),
                |ui| {
                    if theme::toggle(ui, &p, &mut enabled).changed() {
                        match autostart::set(enabled) {
                            Ok(state) => self.autostart = Some(state),
                            Err(err) => error = Some(err),
                        }
                    }
                },
            );
            ui.separator();

            let settings = &mut shared.settings;
            row(
                ui,
                &p,
                t("Beim Schließen im Tray weiterlaufen"),
                t("Aus: Schließen beendet die App vollständig."),
                |ui| {
                    changed |= theme::toggle(ui, &p, &mut settings.close_to_tray).changed();
                },
            );
            ui.separator();

            row(
                ui,
                &p,
                t("Startmodul"),
                t("Welches Werkzeug beim Öffnen angezeigt wird."),
                |ui| {
                    let current = Tab::ALL
                        .into_iter()
                        .find(|tab| tab.key() == settings.start_tab)
                        .map_or(t("Zuletzt verwendet"), Tab::label);
                    egui::ComboBox::from_id_salt("start_tab")
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            changed |= ui
                                .selectable_value(
                                    &mut settings.start_tab,
                                    String::new(),
                                    "Zuletzt verwendet",
                                )
                                .clicked();
                            for tab in Tab::ALL {
                                changed |= ui
                                    .selectable_value(
                                        &mut settings.start_tab,
                                        tab.key().to_owned(),
                                        tab.label(),
                                    )
                                    .clicked();
                            }
                        });
                },
            );
            ui.separator();

            row(
                ui,
                &p,
                t("Lautstärkeregler bei Spotify"),
                t("Regelt die Lautstärke der Spotify-App im Windows-Mixer."),
                |ui| {
                    changed |= theme::toggle(ui, &p, &mut settings.show_volume).changed();
                },
            );
            ui.separator();

            row(
                ui,
                &p,
                t("Systemwerkzeuge im App-Starter"),
                t("Zeigt auch Verwaltungsprogramme, Hilfedateien und Ähnliches."),
                |ui| {
                    changed |= theme::toggle(ui, &p, &mut settings.show_system_apps).changed();
                },
            );
        });

        section(ui, &p, t("Daten"), |ui| {
            let folder = shared.data_dir.display().to_string();
            row(ui, &p, t("Datenordner"), &folder, |ui| {
                if ui.button(t("Ordner öffnen")).clicked() {
                    if let Err(err) = platform::open_folder(&shared.data_dir) {
                        error = Some(err.into());
                    }
                }
            });
            ui.separator();
            let protection = if cfg!(windows) {
                "Verschlüsselt mit dem Windows-Datenschutz – nur dein Windows-Konto kann die Dateien lesen."
            } else {
                "Auf diesem System werden die Dateien unverschlüsselt gespeichert."
            };
            row(ui, &p, t("Schutz der Daten"), protection, |_ui| {});
            ui.separator();
            row(ui, &p, t("Version"), "", |ui| {
                ui.label(theme::secondary(&p, env!("CARGO_PKG_VERSION")));
            });
        });

        if let Some(err) = error {
            shared.notify(err);
        }
        if changed {
            shared.save_settings();
        }
    }
}
