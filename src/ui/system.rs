use eframe::egui::{
    self, Align, Align2, Color32, CursorIcon, Layout, Order, RichText, Rounding, Sense, Stroke, Ui,
};

use crate::i18n::{t, td};
use crate::{
    app::{self, Shared, Tab, TabDrag},
    backup, platform,
    services::autostart,
    settings::{Accent, Appearance, Language, Settings, ZOOM_LEVELS},
    theme::{self, Palette},
    ui::icons,
};

/// Welches Auswahlfenster mit Vorschau gerade offen ist.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Picker {
    Appearance,
    Accent,
    Size,
}

#[derive(Default)]
pub struct SystemView {
    /// Autostart-Zustand wird beim ersten Anzeigen einmal abgefragt.
    autostart: Option<bool>,
    picker: Option<Picker>,
    /// Gewählte Sicherungsdatei, die nach Bestätigung eingespielt wird.
    confirm_import: Option<std::path::PathBuf>,
}

/// Kleine Vorschau einer Oberfläche mit der Palette `p`: Seitenleiste, Fläche, Karte, Akzent.
pub(crate) fn preview(
    ui: &mut Ui,
    size: egui::Vec2,
    p: &Palette,
    selected: Option<Color32>,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let painter = ui.painter();
    let corner = 6.0;
    painter.rect_filled(rect, corner, p.content_bg);
    let side = egui::Rect::from_min_size(rect.min, egui::vec2(rect.width() * 0.27, rect.height()));
    painter.rect_filled(
        side,
        Rounding {
            nw: corner,
            sw: corner,
            ne: 0.0,
            se: 0.0,
        },
        p.sidebar_bg,
    );
    // Seitenleisten-Einträge; der erste ist gewählt.
    for row in 0..3 {
        let y = side.top() + 10.0 + row as f32 * 9.0;
        let bar = egui::Rect::from_min_size(
            egui::pos2(side.left() + 5.0, y),
            egui::vec2(side.width() - 10.0, 5.0),
        );
        painter.rect_filled(bar, 2.0, if row == 0 { p.accent } else { p.text_secondary });
    }
    let card = egui::Rect::from_min_max(
        egui::pos2(side.right() + 7.0, rect.top() + 8.0),
        egui::pos2(rect.right() - 7.0, rect.bottom() - 8.0),
    );
    painter.rect_filled(card, 4.0, p.card_bg);
    painter.rect_stroke(card, 4.0, Stroke::new(1.0_f32, p.separator));
    for line in 0..2 {
        let y = card.top() + 7.0 + line as f32 * 7.0;
        let width = card.width() * if line == 0 { 0.65 } else { 0.45 };
        painter.rect_filled(
            egui::Rect::from_min_size(egui::pos2(card.left() + 6.0, y), egui::vec2(width, 3.5)),
            1.5,
            if line == 0 { p.text } else { p.text_secondary },
        );
    }
    let button = egui::Rect::from_min_size(
        egui::pos2(card.left() + 6.0, card.bottom() - 12.0),
        egui::vec2(card.width() * 0.4, 7.0),
    );
    painter.rect_filled(button, 3.0, p.accent);
    let stroke = match selected {
        Some(color) => Stroke::new(2.5_f32, color),
        None => Stroke::new(1.0_f32, p.separator),
    };
    painter.rect_stroke(rect, corner, stroke);
    response.on_hover_cursor(CursorIcon::PointingHand)
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
        row(
            ui,
            p,
            "Sprache",
            "Sprache der gesamten Oberfläche. Wirkt sofort.",
            |ui| {
                for option in Language::ALL.into_iter().rev() {
                    if theme::chip(ui, p, option.label(), settings.language == option).clicked() {
                        settings.language = option;
                        *changed = true;
                    }
                }
            },
        );
        ui.separator();

        row(
            ui,
            p,
            "Erscheinungsbild",
            "Hell, dunkel, reines Schwarz (OLED), hoher Kontrast oder eigene Farben. Das Fenster zeigt eine Vorschau.",
            |ui| {
                if ui
                    .button(format!("{}  …", t(settings.appearance.label())))
                    .clicked()
                {
                    self.picker = Some(Picker::Appearance);
                }
            },
        );
        if settings.appearance == Appearance::Custom {
            ui.add_space(2.0);
            row(
                ui,
                p,
                "Hintergrund",
                "Farbe der Inhaltsfläche. Der Text passt sich der Helligkeit automatisch an.",
                |ui| {
                    *changed |= color_option(ui, &mut settings.custom_background, p.content_bg);
                },
            );
            row(
                ui,
                p,
                "Seitenleiste",
                "Farbe der Navigation und der Titelleiste.",
                |ui| {
                    *changed |= color_option(ui, &mut settings.custom_sidebar, p.sidebar_bg);
                },
            );
            row(ui, p, "Karten", "Farbe der abgesetzten Flächen.", |ui| {
                *changed |= color_option(ui, &mut settings.custom_card, p.card_bg);
            });
        }
        ui.separator();

        row(
            ui,
            p,
            "Akzentfarbe",
            "Farbe für Auswahl, Schalter und Hervorhebungen. Das Fenster zeigt eine Vorschau.",
            |ui| {
                *changed |= color_option(ui, &mut settings.custom_accent, p.accent);
                let name = if settings.custom_accent.is_some() {
                    "Eigene Farbe"
                } else {
                    settings.accent.label()
                };
                if ui.button(format!("{}  …", t(name))).clicked() {
                    self.picker = Some(Picker::Accent);
                }
            },
        );
        ui.separator();

        row(
            ui,
            p,
            "Ecken",
            "Wie rund Karten und Schaltflächen sind: 0 = eckig, höhere Werte = runder.",
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
            "Größe",
            "Skaliert die gesamte Oberfläche – Text, Abstände und Symbole. Das Fenster zeigt eine Vorschau.",
            |ui| {
                let name = ZOOM_LEVELS
                    .into_iter()
                    .find(|(zoom, _)| (settings.zoom - zoom).abs() < 0.01)
                    .map_or("Eigene", |(_, label)| label);
                if ui.button(format!("{}  …", t(name))).clicked() {
                    self.picker = Some(Picker::Size);
                }
            },
        );
        ui.separator();

        row(
            ui,
            p,
            "Kompakte Darstellung",
            "Enge Abstände, kleinere Schaltflächen und Ränder in allen Menüs – mehr Inhalt auf kleinem Platz.",
            |ui| {
                *changed |= theme::toggle(ui, p, &mut settings.compact).changed();
            },
        );
        ui.separator();

        row(
            ui,
            p,
            "Design zurücksetzen",
            "Stellt Erscheinungsbild, Farben, Ecken, Größe und Kompaktmodus auf die Vorgabe.",
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

    /// Auswahlfenster mit Vorschau für Erscheinungsbild, Akzentfarbe und Größe.
    fn pickers(&mut self, ui: &mut Ui, p: &Palette, settings: &mut Settings, changed: &mut bool) {
        let Some(kind) = self.picker else {
            return;
        };
        let title = match kind {
            Picker::Appearance => t("Erscheinungsbild"),
            Picker::Accent => t("Akzentfarbe"),
            Picker::Size => t("Größe"),
        };
        let mut close = false;
        let system_dark = ui.ctx().theme() == egui::Theme::Dark;
        let tile = egui::vec2(150.0, 88.0);
        egui::Window::new(title)
            .id(egui::Id::new(("toolless-picker", ui.id())))
            .order(Order::Tooltip)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ui.ctx(), |ui| {
                ui.set_max_width(520.0);
                match kind {
                    Picker::Appearance => {
                        ui.horizontal_wrapped(|ui| {
                            for option in Appearance::ALL {
                                let mut sample = settings.clone();
                                sample.appearance = option;
                                let palette = Palette::from_settings(system_dark, &sample);
                                ui.vertical(|ui| {
                                    let current = settings.appearance == option;
                                    if preview(ui, tile, &palette, current.then_some(p.accent))
                                        .clicked()
                                    {
                                        settings.appearance = option;
                                        *changed = true;
                                    }
                                    ui.label(t(option.label()));
                                });
                            }
                        });
                    }
                    Picker::Accent => {
                        ui.horizontal_wrapped(|ui| {
                            for accent in Accent::ALL {
                                let mut sample = settings.clone();
                                sample.accent = accent;
                                sample.custom_accent = None;
                                let palette = Palette::from_settings(system_dark, &sample);
                                ui.vertical(|ui| {
                                    let current = settings.custom_accent.is_none()
                                        && settings.accent == accent;
                                    if preview(ui, tile, &palette, current.then_some(palette.accent))
                                        .clicked()
                                    {
                                        settings.accent = accent;
                                        settings.custom_accent = None;
                                        *changed = true;
                                    }
                                    ui.label(t(accent.label()));
                                });
                            }
                        });
                        ui.add_space(4.0);
                        ui.label(
                            theme::secondary(
                                p,
                                t("Eine eigene Farbe lässt sich im Menü hinter dem Farbfeld mischen."),
                            )
                            .size(12.5),
                        );
                    }
                    Picker::Size => {
                        ui.horizontal_wrapped(|ui| {
                            for (zoom, _) in ZOOM_LEVELS {
                                let (rect, response) =
                                    ui.allocate_exact_size(egui::vec2(120.0, 78.0), Sense::click());
                                let current = (settings.zoom - zoom).abs() < 0.01;
                                ui.painter().rect_filled(rect, 6.0, p.card_bg);
                                let stroke = if current {
                                    Stroke::new(2.5_f32, p.accent)
                                } else {
                                    Stroke::new(1.0_f32, p.separator)
                                };
                                ui.painter().rect_stroke(rect, 6.0, stroke);
                                // Beispieltext in der jeweiligen Größe (relativ zu „Normal“).
                                ui.painter().with_clip_rect(rect.shrink(4.0)).text(
                                    egui::pos2(rect.left() + 10.0, rect.top() + 10.0),
                                    Align2::LEFT_TOP,
                                    td("Beispieltext".to_owned()),
                                    egui::FontId::proportional(14.0 * zoom),
                                    p.text,
                                );
                                ui.painter().with_clip_rect(rect.shrink(4.0)).text(
                                    egui::pos2(rect.left() + 10.0, rect.bottom() - 10.0),
                                    Align2::LEFT_BOTTOM,
                                    format!("{:.0} %", zoom * 100.0),
                                    egui::FontId::proportional(12.0),
                                    p.text_secondary,
                                );
                                if response.on_hover_cursor(CursorIcon::PointingHand).clicked() {
                                    settings.zoom = zoom;
                                    *changed = true;
                                }
                            }
                        });
                    }
                }
                ui.add_space(6.0);
                if theme::primary_button(ui, p, t("Fertig"), true).clicked() {
                    close = true;
                }
            });
        if close || ui.ctx().input(|input| input.key_pressed(egui::Key::Escape)) {
            self.picker = None;
        }
    }

    /// Werkzeuge der Seitenleiste ein-/ausblenden und umsortieren (auch per Ziehen).
    fn tools(&mut self, ui: &mut Ui, p: &Palette, settings: &mut Settings, changed: &mut bool) {
        let tabs = app::ordered_tabs(settings);
        let count = tabs.len();
        let mut dropped: Option<(Tab, Tab)> = None;
        for (index, tab) in tabs.into_iter().enumerate() {
            let hidden = app::is_tab_hidden(settings, tab);
            let line = ui
                .horizontal(|ui| {
                    // Griff zum Ziehen.
                    let (handle_rect, handle) =
                        ui.allocate_exact_size(egui::vec2(18.0, 24.0), Sense::drag());
                    for dy in [-5.0_f32, 0.0, 5.0] {
                        for dx in [-3.0_f32, 3.0] {
                            ui.painter().circle_filled(
                                handle_rect.center() + egui::vec2(dx, dy),
                                1.4,
                                p.text_secondary,
                            );
                        }
                    }
                    handle.dnd_set_drag_payload(TabDrag(tab));
                    if handle.dragged() {
                        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
                    } else if handle.hovered() {
                        ui.ctx().set_cursor_icon(CursorIcon::Grab);
                    }

                    let (icon_rect, _) =
                        ui.allocate_exact_size(egui::vec2(20.0, 20.0), Sense::hover());
                    let color = if hidden { p.text_secondary } else { p.text };
                    icons::draw(ui.painter(), icon_rect.center(), 16.0, tab, color);
                    let label = RichText::new(tab.label());
                    ui.label(if hidden {
                        label.color(p.text_secondary)
                    } else {
                        label.strong()
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let mut visible = !hidden;
                        if theme::toggle(ui, p, &mut visible).changed() {
                            app::set_tab_hidden(settings, tab, !visible);
                            *changed = true;
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
                })
                .response;
            if line
                .dnd_hover_payload::<TabDrag>()
                .is_some_and(|dragged| dragged.0 != tab)
            {
                theme::drop_highlight(ui, p, line.rect);
            }
            if ui.input(|input| input.pointer.any_released()) {
                if let Some(dragged) = line.dnd_release_payload::<TabDrag>() {
                    if dragged.0 != tab {
                        dropped = Some((dragged.0, tab));
                    }
                }
            }
            if index + 1 < count {
                ui.separator();
            }
        }
        if let Some((tab, target)) = dropped {
            app::move_tab_to(settings, tab, target);
            *changed = true;
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, shared: &mut Shared) {
        let p = shared.palette;
        ui.set_max_width(720.0);
        let mut changed = false;
        let mut error = None;

        section(ui, &p, "Design", |ui| {
            self.design(ui, &p, &mut shared.settings, &mut changed);
        });

        section(ui, &p, "Werkzeuge", |ui| {
            ui.label(
                theme::secondary(
                    &p,
                    t("Reihenfolge und Sichtbarkeit in der Seitenleiste. Am Griff links lassen sich die Einträge ziehen; die Einstellungen sind zusätzlich immer über das Zahnrad oben rechts erreichbar."),
                )
                .size(12.5),
            );
            ui.separator();
            self.tools(ui, &p, &mut shared.settings, &mut changed);
        });

        section(ui, &p, "Spotify", |ui| {
            row(
                ui,
                &p,
                "Akzentfarbe vom Cover",
                "Der Spotify-Tab nutzt die Hauptfarbe des aktuellen Covers als Akzentfarbe (Knöpfe, Fortschritt).",
                |ui| {
                    changed |= theme::toggle(ui, &p, &mut shared.settings.spotify_cover_accent)
                        .changed();
                },
            );
        });

        section(ui, &p, "Verhalten", |ui| {
            let mut keep_awake = shared.power.is_active();
            row(
                ui,
                &p,
                "Ruhezustand blockieren",
                "Hält den Rechner und den Bildschirm wach, solange der Schalter an ist. AFK- und Vollbildmodi tun das automatisch.",
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
                "Mit dem System starten",
                "Startet Toolless beim Anmelden automatisch, versteckt im System-Tray.",
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
                "Beim Schließen im Tray weiterlaufen",
                "An: Das Schließen versteckt nur das Fenster (Wecker und Aufzeichnung laufen weiter). Aus: Die App wird beendet.",
                |ui| {
                    changed |= theme::toggle(ui, &p, &mut settings.close_to_tray).changed();
                },
            );
            ui.separator();

            row(
                ui,
                &p,
                "Startmodul",
                "Welches Werkzeug beim Öffnen zuerst angezeigt wird.",
                |ui| {
                    let current = Tab::ALL
                        .into_iter()
                        .filter(|tab| *tab != Tab::System)
                        .find(|tab| tab.key() == settings.start_tab)
                        .map_or(t("Zuletzt verwendet"), Tab::label);
                    egui::ComboBox::from_id_salt(ui.id().with("start_tab"))
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            changed |= ui
                                .selectable_value(
                                    &mut settings.start_tab,
                                    String::new(),
                                    t("Zuletzt verwendet"),
                                )
                                .clicked();
                            for tab in Tab::ALL.into_iter().filter(|tab| *tab != Tab::System) {
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
                "Systemwerkzeuge im App-Starter",
                "Zeigt im App-Starter auch Verwaltungsprogramme, Hilfedateien und Deinstallationsprogramme.",
                |ui| {
                    changed |= theme::toggle(ui, &p, &mut settings.show_system_apps).changed();
                },
            );
        });

        section(ui, &p, "Daten", |ui| {
            let folder = shared.data_dir.display().to_string();
            row(
                ui,
                &p,
                "Datenordner",
                &format!(
                    "{} {folder}",
                    t("Hier liegen Einstellungen, Zwischenablage, Notizen und Verlauf:")
                ),
                |ui| {
                    if ui.button(t("Ordner öffnen")).clicked() {
                        if let Err(err) = platform::open_folder(&shared.data_dir) {
                            error = Some(err.into());
                        }
                    }
                },
            );
            ui.separator();

            row(
                ui,
                &p,
                "Sicherung",
                "Exportiert alles – Einstellungen, Zwischenablage, Notizen mit Bildern, Verlauf und Spielzeiten – in eine Datei, da es keine Konten gibt. Zum Umziehen auf einen anderen Rechner wieder importieren.",
                |ui| {
                    if ui.button(t("Importieren …")).clicked() {
                        let picked = rfd::FileDialog::new()
                            .add_filter("Toolless", &["toolless", "json"])
                            .pick_file();
                        if let Some(path) = picked {
                            self.confirm_import = Some(path);
                        }
                    }
                    if ui.button(t("Exportieren …")).clicked() {
                        let picked = rfd::FileDialog::new()
                            .set_file_name("toolless-sicherung.toolless")
                            .add_filter("Toolless", &["toolless"])
                            .save_file();
                        if let Some(path) = picked {
                            match backup::export(&shared.data_dir, &path) {
                                Ok(count) => shared.notify(format!(
                                    "Sicherung gespeichert ({count} Dateien). Bewahre sie sicher auf – sie ist nicht verschlüsselt."
                                )),
                                Err(err) => error = Some(err),
                            }
                        }
                    }
                },
            );
            if let Some(path) = self.confirm_import.clone() {
                ui.add_space(4.0);
                theme::card(&p).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        RichText::new(t("Sicherung wirklich einspielen?"))
                            .strong()
                            .color(p.accent),
                    );
                    ui.label(
                        theme::secondary(
                            &p,
                            t("Alle aktuellen Daten werden ersetzt und Toolless wird danach beendet. Bitte anschließend neu starten."),
                        )
                        .size(12.5),
                    );
                    ui.horizontal(|ui| {
                        if theme::primary_button(ui, &p, t("Importieren und beenden"), true)
                            .clicked()
                        {
                            match backup::import(&shared.data_dir, &path) {
                                // Die laufende App würde die Daten sonst wieder überschreiben.
                                Ok(_) => std::process::exit(0),
                                Err(err) => {
                                    error = Some(err);
                                    self.confirm_import = None;
                                }
                            }
                        }
                        if ui.button(t("Abbrechen")).clicked() {
                            self.confirm_import = None;
                        }
                    });
                });
            }
            ui.separator();

            let protection = if cfg!(windows) {
                "Verschlüsselt mit dem Windows-Datenschutz – nur dein Windows-Konto kann die Dateien lesen."
            } else {
                "Auf diesem System werden die Dateien unverschlüsselt gespeichert."
            };
            row(ui, &p, "Schutz der Daten", protection, |_ui| {});
            ui.separator();
            row(ui, &p, "Version", "", |ui| {
                ui.label(theme::secondary(&p, env!("CARGO_PKG_VERSION")));
            });
        });

        self.pickers(ui, &p, &mut shared.settings, &mut changed);

        if let Some(err) = error {
            shared.notify(err);
        }
        if changed {
            shared.save_settings();
        }
    }
}
