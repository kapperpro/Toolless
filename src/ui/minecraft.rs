//! Minecraft: Welten mit Seed und Version, MCA Selector und die Seed-Karten.

use crate::i18n::t;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use eframe::egui::{self, Align, Button, Layout, RichText, Sense, TextEdit, TextureHandle, Ui};

use crate::{
    app::Shared,
    platform,
    services::minecraft::{self, SeedMap, World},
    theme::{self, Palette},
};

enum Action {
    OpenMap(SeedMap, Option<i64>, String),
    CopySeed(i64),
    CopyPath(PathBuf),
    OpenFolder(PathBuf),
    /// MCA Selector starten, optional gleich mit dieser Welt.
    LaunchMca(Option<PathBuf>),

    PickMca,
    DownloadMca,
    AddFolder,
    RemoveFolder(usize),
    Refresh,
    SaveSeed(String, String, String),
    RemoveSeed(usize),
}

/// Untertabs des Minecraft-Moduls.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Sub {
    #[default]
    Mca,
    SeedMap,
}

#[derive(Default)]
pub struct MinecraftView {
    loaded: bool,
    worlds: Vec<World>,
    mca_selector: Option<PathBuf>,
    search: String,
    seed_input: String,
    version_input: String,
    name_input: String,
    sub: Sub,

    /// Vorschaubilder der Welten; `None` = Datei ließ sich nicht lesen.
    icons: HashMap<PathBuf, Option<TextureHandle>>,
}

/// Kantenlänge des Weltbilds in der Liste.
const WORLD_ICON: f32 = 56.0;
/// So viele Vorschaubilder werden höchstens je Bild geladen, damit die Liste nicht stockt.
const ICONS_PER_FRAME: usize = 3;

fn load_icon(ui: &Ui, path: &Path) -> Option<TextureHandle> {
    let image = image::open(path).ok()?.to_rgba8();
    let size = [image.width() as usize, image.height() as usize];
    let pixels = egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw());
    Some(ui.ctx().load_texture(
        format!("world-icon:{}", path.display()),
        pixels,
        egui::TextureOptions::NEAREST,
    ))
}

/// „heute“, „vor 3 Tagen“ … aus einem Zeitstempel in Millisekunden.
fn format_last_played(ms: i64) -> String {
    if ms <= 0 {
        return String::new();
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    match (now - ms).max(0) / 86_400_000 {
        0 => "heute gespielt".to_owned(),
        1 => "gestern gespielt".to_owned(),
        days @ 2..=59 => crate::i18n::td(format!("vor {days} Tagen")),
        days @ 60..=729 => crate::i18n::td(format!("vor {} Monaten", days / 30)),
        days => crate::i18n::td(format!("vor {} Jahren", days / 365)),
    }
}

fn td_seed_name(seed: &str) -> String {
    let seed = seed.trim();
    if seed.chars().count() > 18 {
        format!("{}…", seed.chars().take(18).collect::<String>())
    } else {
        format!("Seed {seed}")
    }
}

/// Die drei Karten-Schaltflächen für einen Seed.
fn map_buttons(ui: &mut Ui, seed: Option<i64>, version: &str, action: &mut Option<Action>) {
    for map in SeedMap::ALL {
        if ui.button(map.label()).on_hover_text(map.hint()).clicked() {
            *action = Some(Action::OpenMap(map, seed, version.to_owned()));
        }
    }
}

impl MinecraftView {
    pub fn reload(&mut self, shared: &Shared) {
        self.worlds = minecraft::scan_worlds(&shared.settings.minecraft_folders);
        self.mca_selector = minecraft::find_mca_selector(&shared.settings.mca_selector_path);
    }

    fn apply(&mut self, action: Action, ui: &Ui, shared: &mut Shared) {
        match action {
            Action::OpenMap(map, seed, version) => {
                let url = map.url(seed, &version);
                match platform::open_url(&url) {
                    Ok(()) => {
                        // Nicht jede Seite übernimmt den Seed aus der Adresse – er liegt
                        // deshalb zusätzlich zum Einfügen bereit.
                        if let Some(seed) = seed {
                            ui.ctx().copy_text(seed.to_string());
                            shared.notify(crate::i18n::td(format!("{} geöffnet · Seed kopiert", map.label())));
                        }
                    }
                    Err(err) => shared.notify(crate::i18n::td(format!("Browser ließ sich nicht öffnen: {err}"))),
                }
            }
            Action::CopySeed(seed) => {
                ui.ctx().copy_text(seed.to_string());
                shared.notify(t("Seed kopiert"));
            }
            Action::CopyPath(path) => {
                ui.ctx().copy_text(path.to_string_lossy().into_owned());
                shared.notify(t("Pfad der Welt kopiert"));
            }
            Action::OpenFolder(path) => {
                if let Err(err) = platform::open_folder(&path) {
                    shared.notify(crate::i18n::td(format!("Ordner ließ sich nicht öffnen: {err}")));
                }
            }
                        Action::LaunchMca(world) => match &self.mca_selector {
                Some(path) => match minecraft::launch_mca_selector(path, world.as_deref()) {
                    Ok(true) => shared.notify(
                        t("MCA Selector startet – die Welt wird gleich geöffnet, bitte kurz nichts anklicken."),
                    ),
                    Ok(false) => {}
                    Err(err) => shared.notify(err),
                },
                None => shared.notify(t("MCA Selector wurde nicht gefunden – bitte Pfad wählen.")),
            },
            Action::PickMca => {
                let picked = rfd::FileDialog::new()
                    .set_title(t("MCA Selector auswählen"))
                    .add_filter("MCA Selector", &["exe", "jar"])
                    .pick_file();
                if let Some(path) = picked {
                    shared.settings.mca_selector_path = path.to_string_lossy().into_owned();
                    shared.save_settings();
                    self.mca_selector = Some(path);
                }
            }
            Action::DownloadMca => {
                if let Err(err) = platform::open_url(minecraft::MCA_SELECTOR_DOWNLOAD) {
                    shared.notify(crate::i18n::td(format!("Browser ließ sich nicht öffnen: {err}")));
                }
            }
            Action::AddFolder => {
                let picked = rfd::FileDialog::new()
                    .set_title(t("Minecraft-Ordner, Instanz oder saves-Ordner wählen"))
                    .pick_folder();
                if let Some(folder) = picked {
                    let folder = folder.to_string_lossy().into_owned();
                    if !shared.settings.minecraft_folders.contains(&folder) {
                        shared.settings.minecraft_folders.push(folder);
                        shared.save_settings();
                    }
                    self.reload(shared);
                }
            }
            Action::RemoveFolder(index) => {
                if index < shared.settings.minecraft_folders.len() {
                    shared.settings.minecraft_folders.remove(index);
                    shared.save_settings();
                    self.reload(shared);
                }
            }
                        Action::SaveSeed(name, seed, version) => {
                let name = if name.trim().is_empty() {
                    td_seed_name(&seed)
                } else {
                    name.trim().to_owned()
                };
                shared.settings.minecraft_seeds.push((name, seed, version));
                shared.save_settings();
                self.name_input.clear();
                shared.notify(t("Seed gemerkt"));
            }
            Action::RemoveSeed(index) => {
                if index < shared.settings.minecraft_seeds.len() {
                    shared.settings.minecraft_seeds.remove(index);
                    shared.save_settings();
                }
            }
            Action::Refresh => {
                self.reload(shared);
                shared.notify(crate::i18n::td(format!("{} Welten gefunden", self.worlds.len())));
            }
        }
    }

    fn mca_card(&mut self, ui: &mut Ui, p: &Palette, action: &mut Option<Action>) {
        // ── MCA Selector ──
        theme::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_max_width((ui.available_width() - 330.0).max(200.0));
                    theme::card_title(ui, "MCA Selector");
                    let status = match &self.mca_selector {
                        Some(path) => path.to_string_lossy().into_owned(),
                        None => "Nicht gefunden – Pfad wählen oder herunterladen.".to_owned(),
                    };
                    ui.add(egui::Label::new(theme::secondary(p, status).size(12.5)).truncate());
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button(t("Herunterladen")).clicked() {
                        *action = Some(Action::DownloadMca);
                    }
                    if ui.button(t("Pfad wählen …")).clicked() {
                        *action = Some(Action::PickMca);
                    }
                    if theme::primary_button(ui, p, t("Starten"), self.mca_selector.is_some()).clicked() {
                        *action = Some(Action::LaunchMca(None));
                    }
                });
            });
            ui.label(
                theme::secondary(
                    p,
                    t("Chunks ansehen, filtern, löschen und exportieren. Die Schaltfläche „MCA Selector“ bei \
                     einer Welt startet das Programm und öffnet die Welt darin."),
                )
                .size(12.5),
            );
        });
    }

    /// Zeichnet das Vorschaubild der Welt oder einen Platzhalter.
    fn world_icon(&self, ui: &mut Ui, p: &Palette, world: &World) {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(WORLD_ICON, WORLD_ICON), Sense::hover());
        let texture = world
            .icon
            .as_ref()
            .and_then(|path| self.icons.get(path))
            .and_then(Option::as_ref);
        match texture {
            Some(texture) => {
                let uv = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                ui.painter()
                    .image(texture.id(), rect, uv, egui::Color32::WHITE);
            }
            None => {
                ui.painter().rect_filled(rect, p.radius, p.field_bg);
                let letter: String = world
                    .name
                    .chars()
                    .take(1)
                    .collect::<String>()
                    .to_uppercase();
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    letter,
                    egui::FontId::proportional(24.0),
                    p.text_secondary,
                );
            }
        }
    }

    fn world_card(&self, ui: &mut Ui, p: &Palette, world: &World, action: &mut Option<Action>) {
        theme::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                self.world_icon(ui, p, world);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&world.name).strong().size(15.0));
                        if !world.version.is_empty() {
                            theme::badge(ui, p, &world.version);
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                theme::secondary(p, format_last_played(world.last_played_ms))
                                    .size(12.5),
                            );
                        });
                    });

                    // Herkunft: Modpack bzw. Instanz, Mod-Lader, Anzahl der Mods, Launcher.
                    let mut origin: Vec<String> = Vec::new();
                    if world.modded || world.mods > 0 {
                        let loader = if world.brands.is_empty() {
                            "Mods".to_owned()
                        } else {
                            world.brands.join(", ")
                        };
                        origin.push(match world.mods {
                            0 => loader,
                            count => crate::i18n::td(format!("{loader} · {count} Mods")),
                        });
                    } else {
                        origin.push(t("Vanilla").to_owned());
                    }
                    origin.push(world.source.clone());
                    ui.horizontal(|ui| {
                        if !world.pack.is_empty() {
                            ui.label(RichText::new(&world.pack).color(p.accent).strong());
                        }
                        ui.label(theme::secondary(p, origin.join(" · ")).size(12.5));
                    });

                    let seed_text = world.seed.map_or_else(
                        || t("Seed nicht gefunden").to_owned(),
                        |seed| format!("Seed {seed}"),
                    );
                    let datapacks = match world.datapacks.len() {
                        0 => String::new(),
                        count => crate::i18n::td(format!(" · {count} Datapacks")),
                    };
                    let line =
                        ui.label(theme::secondary(p, format!("{seed_text}{datapacks}")).size(12.5));
                    if !world.datapacks.is_empty() {
                        line.on_hover_text(world.datapacks.join("\n"));
                    }
                });
            });
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                if let Some(seed) = world.seed {
                    if ui.button(t("Seed kopieren")).clicked() {
                        *action = Some(Action::CopySeed(seed));
                    }
                }
                if ui.button(t("Ordner öffnen")).clicked() {
                    *action = Some(Action::OpenFolder(world.folder.clone()));
                }
                if ui.button(t("Pfad kopieren")).clicked() {
                    *action = Some(Action::CopyPath(world.folder.clone()));
                }
                if ui
                    .add_enabled(self.mca_selector.is_some(), Button::new("MCA Selector"))
                    .on_hover_text(t("Startet MCA Selector und öffnet diese Welt darin"))
                    .clicked()
                {
                    // Falls das automatische Öffnen nicht greift, liegt der Pfad bereit.
                    ui.ctx()
                        .copy_text(world.folder.to_string_lossy().into_owned());
                    *action = Some(Action::LaunchMca(Some(world.folder.clone())));
                }
            });
        });
    }

    /// Kompakte Zeile einer Welt im Seed-Map-Tab: Seed und die Karten-Schaltflächen.
    fn seed_world_row(&self, ui: &mut Ui, p: &Palette, world: &World, action: &mut Option<Action>) {
        theme::card(p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                self.world_icon(ui, p, world);
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&world.name).strong().size(15.0));
                        if !world.version.is_empty() {
                            theme::badge(ui, p, &world.version);
                        }
                    });
                    if !world.pack.is_empty() {
                        ui.label(RichText::new(&world.pack).color(p.accent));
                    }
                    let seed_text = world.seed.map_or_else(
                        || t("Seed nicht gefunden").to_owned(),
                        |seed| format!("Seed {seed}"),
                    );
                    ui.label(theme::secondary(p, seed_text).size(12.5));
                });
            });
            if let Some(seed) = world.seed {
                ui.horizontal_wrapped(|ui| {
                    if ui.button(t("Seed kopieren")).clicked() {
                        *action = Some(Action::CopySeed(seed));
                    }
                    if ui
                        .button(t("Seed merken"))
                        .on_hover_text(t("Legt den Seed in „Meine Seeds“ ab"))
                        .clicked()
                    {
                        *action = Some(Action::SaveSeed(
                            world.name.clone(),
                            seed.to_string(),
                            world.version.clone(),
                        ));
                    }
                    map_buttons(ui, Some(seed), &world.version, action);
                });
            }
        });
    }

    /// Suchfeld und Liste der Welten (für beide Untertabs).
    fn worlds_section(&mut self, ui: &mut Ui, shared: &Shared, action: &mut Option<Action>) {
        let p = shared.palette;
        let compact = self.sub == Sub::SeedMap;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(crate::i18n::td(format!("Welten ({})", self.worlds.len()))).strong(),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if !compact {
                    if ui
                        .button(t("Ordner hinzufügen …"))
                        .on_hover_text(t(
                            "Für Launcher oder Instanzen, die nicht automatisch gefunden werden",
                        ))
                        .clicked()
                    {
                        *action = Some(Action::AddFolder);
                    }
                }
                if ui.button(t("Aktualisieren")).clicked() {
                    *action = Some(Action::Refresh);
                }
                ui.add(
                    TextEdit::singleline(&mut self.search)
                        .hint_text(t("Welt, Modpack, Version oder Launcher suchen"))
                        .desired_width(if compact { 150.0 } else { 240.0 }),
                );
            });
        });
        if !shared.settings.minecraft_folders.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label(theme::secondary(&p, t("Eigene Ordner:")).size(12.5));
                for (index, folder) in shared.settings.minecraft_folders.iter().enumerate() {
                    if ui
                        .small_button(format!("{folder}  ×"))
                        .on_hover_text(t("Ordner aus der Liste entfernen"))
                        .clicked()
                    {
                        *action = Some(Action::RemoveFolder(index));
                    }
                }
            });
        }

        let needle = self.search.trim().to_lowercase();
        let visible: Vec<&World> = self
            .worlds
            .iter()
            .filter(|world| {
                needle.is_empty()
                    || world.name.to_lowercase().contains(&needle)
                    || world.version.to_lowercase().contains(&needle)
                    || world.source.to_lowercase().contains(&needle)
                    || world.pack.to_lowercase().contains(&needle)
            })
            .collect();
        if self.worlds.is_empty() {
            theme::empty_state(
                ui,
                &p,
                t("Keine Welten gefunden. Füge den Ordner deines Launchers über „Ordner hinzufügen …“ hinzu."),
            );
        } else if visible.is_empty() {
            theme::empty_state(ui, &p, t("Keine Welt passt zur Suche."));
        }
        // Vorschaubilder nach und nach laden.
        let missing: Vec<PathBuf> = visible
            .iter()
            .filter_map(|world| world.icon.clone())
            .filter(|path| !self.icons.contains_key(path))
            .take(ICONS_PER_FRAME)
            .collect();
        let more_to_load = missing.len() == ICONS_PER_FRAME;
        let loaded: Vec<(PathBuf, Option<TextureHandle>)> = missing
            .into_iter()
            .map(|path| {
                let texture = load_icon(ui, &path);
                (path, texture)
            })
            .collect();

        for world in visible {
            if compact {
                self.seed_world_row(ui, &p, world, action);
            } else {
                self.world_card(ui, &p, world, action);
            }
        }

        self.icons.extend(loaded);
        if more_to_load {
            ui.ctx().request_repaint();
        }
    }

    /// Seed eintippen, Karten öffnen und Seeds merken.
    fn seeds_card(&mut self, ui: &mut Ui, shared: &Shared, action: &mut Option<Action>) {
        let p = shared.palette;
        theme::card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::card_title(ui, t("Seed-Karte"));
            ui.add(
                TextEdit::singleline(&mut self.seed_input)
                    .hint_text(t("Seed (Zahl oder Text)"))
                    .desired_width(f32::INFINITY),
            );
            ui.horizontal(|ui| {
                ui.add(
                    TextEdit::singleline(&mut self.version_input)
                        .hint_text(t("Version, z. B. 1.21.4"))
                        .desired_width(130.0),
                );
                ui.add(
                    TextEdit::singleline(&mut self.name_input)
                        .hint_text(t("Name (optional)"))
                        .desired_width(f32::INFINITY),
                );
            });
            let seed = minecraft::seed_from_text(&self.seed_input);
            ui.horizontal_wrapped(|ui| {
                map_buttons(ui, seed, self.version_input.trim(), action);
            });
            let can_save = !self.seed_input.trim().is_empty();
            if ui
                .add_enabled(can_save, Button::new(t("Seed merken")))
                .clicked()
            {
                *action = Some(Action::SaveSeed(
                    self.name_input.clone(),
                    self.seed_input.trim().to_owned(),
                    self.version_input.trim().to_owned(),
                ));
            }
        });

        theme::card(&p).show(ui, |ui| {
            ui.set_width(ui.available_width());
            theme::card_title(ui, t("Meine Seeds"));
            if shared.settings.minecraft_seeds.is_empty() {
                ui.label(
                    theme::secondary(&p, t("Noch keine Seeds gemerkt. Gib oben einen Seed ein oder nimm ihn von einer Welt."))
                        .size(12.5),
                );
            }
            for (index, (name, seed_text, version)) in
                shared.settings.minecraft_seeds.iter().enumerate()
            {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(name).strong());
                    if !version.is_empty() {
                        theme::badge(ui, &p, version);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .small_button("×")
                            .on_hover_text(t("Seed vergessen"))
                            .clicked()
                        {
                            *action = Some(Action::RemoveSeed(index));
                        }
                    });
                });
                ui.label(theme::secondary(&p, format!("Seed {seed_text}")).size(12.5));
                let seed = minecraft::seed_from_text(seed_text);
                ui.horizontal_wrapped(|ui| {
                    if let Some(seed) = seed {
                        if ui.button(t("Seed kopieren")).clicked() {
                            *action = Some(Action::CopySeed(seed));
                        }
                    }
                    map_buttons(ui, seed, version, action);
                });
                if index + 1 < shared.settings.minecraft_seeds.len() {
                    ui.separator();
                }
            }
        });
    }

    /// Unterseiten-Wahl; steht fest oben, außerhalb des Scrollbereichs.
    pub fn header(&mut self, ui: &mut Ui, shared: &mut Shared) {
        let p = shared.palette;
        if !self.loaded {
            self.loaded = true;
            self.reload(shared);
        }
        ui.horizontal(|ui| {
            for (sub, label) in [(Sub::Mca, "MCA"), (Sub::SeedMap, "Seed Map")] {
                if theme::chip(ui, &p, label, self.sub == sub).clicked() {
                    self.sub = sub;
                }
            }
        });
        ui.add_space(4.0);
    }

    pub fn ui(&mut self, ui: &mut Ui, shared: &mut Shared) {
        let p = shared.palette;
        if !self.loaded {
            self.loaded = true;
            self.reload(shared);
        }
        let mut action = None;

        match self.sub {
            Sub::Mca => {
                self.mca_card(ui, &p, &mut action);
                ui.add_space(4.0);
                self.worlds_section(ui, shared, &mut action);
            }
            Sub::SeedMap => {
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(400.0);
                        self.seeds_card(ui, shared, &mut action);
                        ui.add_space(4.0);
                        self.worlds_section(ui, shared, &mut action);
                    });
                    ui.vertical(|ui| {
                        theme::card(&p).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            theme::card_title(ui, t("Seed-Karte in Toolless"));
                            ui.label(
                                theme::secondary(
                                    &p,
                                    t("Die Karte direkt in Toolless folgt später. Bis dahin öffnen die Schaltflächen links die Seed-Karte im Browser."),
                                )
                                .size(12.5),
                            );
                        });
                    });
                });
            }
        }

        if let Some(action) = action {
            self.apply(action, ui, shared);
        }
    }
}
