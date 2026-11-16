//! Hauptfenster: Titelleiste mit Fensterknöpfen, Seitenleiste, Modulwechsel,
//! Hintergrundbetrieb (Schließen = in den Tray verstecken).

use crate::i18n::t;
use std::{
    fmt::Display,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use eframe::egui::{
    self, viewport::ResizeDirection, Align, Align2, CentralPanel, Color32, CursorIcon, Frame, Id,
    LayerId, Layout, Margin, Order, PointerButton, Rect, RichText, ScrollArea, Sense, SidePanel,
    Stroke, TopBottomPanel, Ui, ViewportCommand,
};

use crate::{
    db::Db,
    error::{AppError, AppResult},
    platform::{self, NativeWindow},
    services::{
        monitor::MonitorHandle, notes::NoteStore, power::PowerState, spotify::SpotifyHandle,
    },
    settings::{Appearance, Language, Settings},
    theme::{self, Palette},
    tray::{self, Tray, TrayDeps},
    ui::{
        calculator::CalculatorView, clipboard::ClipboardView, clock::ClockView, games::GamesView,
        launcher::LauncherView, minecraft::MinecraftView, monitor::MonitorView, notes::NotesView,
        spotify::SpotifyView, system::SystemView,
    },
};

const TOAST_SECONDS: f64 = 3.2;
/// Breite des unsichtbaren Rands, an dem sich das Fenster in der Größe ziehen lässt.
const RESIZE_MARGIN: f32 = 5.0;
/// Obergrenze der Bildrate: 60 Bilder pro Sekunde.
const MIN_FRAME_TIME: Duration = Duration::from_micros(16_667);
const TITLE_BAR_HEIGHT: f32 = 32.0;

/// Zustand und Dienste, die allen Ansichten zur Verfügung stehen.
pub struct Shared {
    pub palette: Palette,
    pub db: Arc<Db>,
    pub power: Arc<PowerState>,
    pub spotify: SpotifyHandle,
    pub monitor: MonitorHandle,
    pub notes: NoteStore,
    pub settings: Settings,
    /// Ordner mit Datenspeicher und Notizen.
    pub data_dir: PathBuf,
    /// Laufzeit in Sekunden (egui-Uhr), pro Frame aktualisiert.
    pub now: f64,
    /// Zählt Änderungen an den Einstellungen – das Design wird dann neu angewendet.
    settings_version: u64,
    toast: Option<(String, f64)>,
}

impl Shared {
    /// Zeigt eine kurze Meldung am Fensterrand.
    pub fn notify(&mut self, message: impl Display) {
        self.toast = Some((
            crate::i18n::td(message.to_string()),
            self.now + TOAST_SECONDS,
        ));
    }

    /// Speichert die Einstellungen und wendet sie im nächsten Frame an.
    pub fn save_settings(&mut self) {
        self.settings_version += 1;
        if let Err(err) = self.settings.save(&self.db) {
            self.notify(err);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Spotify,
    Clipboard,
    Notes,
    Calculator,
    Launcher,
    Monitor,
    Minecraft,
    Games,
    Clock,
    System,
}

impl Tab {
    pub const ALL: [Tab; 10] = [
        Tab::Spotify,
        Tab::Clipboard,
        Tab::Notes,
        Tab::Calculator,
        Tab::Launcher,
        Tab::Monitor,
        Tab::Minecraft,
        Tab::Games,
        Tab::Clock,
        Tab::System,
    ];

    /// Stabiler Schlüssel für die Einstellungen.
    pub fn key(self) -> &'static str {
        match self {
            Tab::Spotify => "spotify",
            Tab::Clipboard => "clipboard",
            Tab::Notes => "notes",
            Tab::Calculator => "calculator",
            Tab::Launcher => "launcher",
            Tab::Monitor => "monitor",
            Tab::Minecraft => "minecraft",
            Tab::Games => "games",
            Tab::Clock => "clock",
            Tab::System => "system",
        }
    }

    fn from_key(key: &str) -> Option<Tab> {
        Tab::ALL.into_iter().find(|tab| tab.key() == key)
    }

    pub fn label(self) -> &'static str {
        crate::i18n::t(self.german_label())
    }

    fn german_label(self) -> &'static str {
        match self {
            Tab::Spotify => "Spotify",
            Tab::Clipboard => "Zwischenablage",
            Tab::Notes => "Notizen",
            Tab::Calculator => "Rechner",
            Tab::Launcher => "App-Starter",
            Tab::Monitor => "Monitor",
            Tab::Minecraft => "Minecraft",
            Tab::Games => "Game Timer",
            Tab::Clock => "Uhr",
            Tab::System => "Einstellungen",
        }
    }

    /// Die Einstellungen lassen sich nicht ausblenden – sonst käme man nicht mehr zurück.
    pub fn can_hide(self) -> bool {
        self != Tab::System
    }
}

// ───────── Reihenfolge und Sichtbarkeit der Werkzeuge ─────────

/// Alle Werkzeuge in der vom Nutzer gewählten Reihenfolge.
pub fn ordered_tabs(settings: &Settings) -> Vec<Tab> {
    let mut tabs = Vec::new();
    let saved = settings
        .tab_order
        .iter()
        .filter_map(|key| Tab::from_key(key));
    for tab in saved.chain(Tab::ALL) {
        if !tabs.contains(&tab) {
            tabs.push(tab);
        }
    }
    tabs
}

pub fn is_tab_hidden(settings: &Settings, tab: Tab) -> bool {
    tab.can_hide() && settings.hidden_tabs.iter().any(|key| key == tab.key())
}

pub fn set_tab_hidden(settings: &mut Settings, tab: Tab, hidden: bool) {
    settings.hidden_tabs.retain(|key| key != tab.key());
    if hidden && tab.can_hide() {
        settings.hidden_tabs.push(tab.key().to_owned());
    }
}

/// Verschiebt ein Werkzeug um eine sichtbare Position nach oben (`-1`) oder unten (`1`).
pub fn move_tab(settings: &mut Settings, tab: Tab, direction: isize) {
    let mut tabs = ordered_tabs(settings);
    let Some(mut index) = tabs.iter().position(|candidate| *candidate == tab) else {
        return;
    };
    loop {
        let target = index as isize + direction;
        if target < 0 || target as usize >= tabs.len() {
            break;
        }
        let target = target as usize;
        tabs.swap(index, target);
        // Ausgeblendete Nachbarn überspringen, damit sich in der Seitenleiste sichtbar etwas bewegt.
        let passed = tabs[index];
        index = target;
        if !is_tab_hidden(settings, passed) {
            break;
        }
    }
    settings.tab_order = tabs.iter().map(|tab| tab.key().to_owned()).collect();
}

/// Setzt ein Werkzeug an die Stelle eines anderen (Ziehen und Ablegen in der Seitenleiste).
pub fn move_tab_to(settings: &mut Settings, tab: Tab, target: Tab) {
    let mut tabs = ordered_tabs(settings);
    let (Some(from), Some(to)) = (
        tabs.iter().position(|candidate| *candidate == tab),
        tabs.iter().position(|candidate| *candidate == target),
    ) else {
        return;
    };
    let moved = tabs.remove(from);
    tabs.insert(to, moved);
    settings.tab_order = tabs.iter().map(|tab| tab.key().to_owned()).collect();
}

enum SidebarAction {
    Move(Tab, isize),
    MoveTo(Tab, Tab),
    Hide(Tab),
}

/// Nutzlast beim Ziehen eines Werkzeugs in der Seitenleiste.
struct TabDrag(Tab);

pub struct App {
    tab: Tab,
    /// Für welche Kombination (System dunkel, Einstellungsstand) der Stil zuletzt gesetzt wurde.
    styled_for: Option<(bool, u64)>,
    /// Einstellungsstand, für den Design-Modus und Zoom zuletzt gesetzt wurden.
    applied_version: Option<u64>,
    window: NativeWindow,
    tray: Option<Tray>,
    tray_keep_awake: bool,
    quit: Arc<AtomicBool>,
    last_frame: Instant,
    shared: Shared,
    spotify_view: SpotifyView,
    clipboard_view: ClipboardView,
    notes_view: NotesView,
    calculator_view: CalculatorView,
    launcher_view: LauncherView,
    monitor_view: MonitorView,
    minecraft_view: MinecraftView,
    games_view: GamesView,
    clock_view: ClockView,

    system_view: SystemView,
}

/// Stellt die Windows-Systemschrift vor die eingebauten Schriften.
fn install_system_font(ctx: &egui::Context) {
    let Some(bytes) = platform::system_font() else {
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts
        .font_data
        .insert("system-ui".to_owned(), egui::FontData::from_owned(bytes));
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, "system-ui".to_owned());
    ctx.set_fonts(fonts);
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, visible: bool) -> AppResult<Self> {
        let data_dir = dirs::data_dir()
            .ok_or_else(|| AppError::other("Datenordner des Systems nicht gefunden"))?
            .join("Toolless");
        std::fs::create_dir_all(&data_dir)?;

        let db = Arc::new(Db::open(&data_dir.join("toolless.dat"))?);
        let power = Arc::new(PowerState::default());
        let window = NativeWindow::new(cc, visible);
        let quit = Arc::new(AtomicBool::new(false));
        let spotify = SpotifyHandle::spawn(cc.egui_ctx.clone());
        install_system_font(&cc.egui_ctx);

        let settings = Settings::load(&db);
        // Der Monitor zeichnet ab jetzt im Hintergrund auf.
        let monitor = MonitorHandle::spawn(
            data_dir.join("history.dat"),
            settings.power_config(),
            settings.game_config(),
            cc.egui_ctx.clone(),
        );
        let start_key = if settings.start_tab.is_empty() {
            &settings.last_tab
        } else {
            &settings.start_tab
        };
        let tab = Tab::from_key(start_key).unwrap_or(Tab::Spotify);

        // Unter Linux bräuchte das Tray eine eigene GTK-Schleife – dort läuft die App ohne.
        let tray = if cfg!(target_os = "linux") {
            None
        } else {
            tray::create(TrayDeps {
                ctx: cc.egui_ctx.clone(),
                window: window.clone(),
                power: Arc::clone(&power),
                spotify: spotify.sender(),
                quit: Arc::clone(&quit),
            })
            .map_err(|err| eprintln!("{err}"))
            .ok()
        };
        if tray.is_none() && !visible {
            // Ohne Tray gäbe es keinen Weg zurück zum Fenster.
            window.show(&cc.egui_ctx);
        }

        // Programme schon beim Start suchen – der App-Starter ist dann sofort gefüllt.
        crate::i18n::set_english(settings.language == Language::English);
        let clock_view = ClockView::new(
            &settings.alarms,
            settings.countdown_secs,
            cc.egui_ctx.clone(),
        );
        let mut launcher_view = LauncherView::default();
        launcher_view.start_scan(&cc.egui_ctx);

        Ok(Self {
            tab,
            styled_for: None,
            applied_version: None,
            window,
            tray,
            tray_keep_awake: false,
            quit,
            last_frame: Instant::now(),
            shared: Shared {
                palette: Palette::from_settings(false, &settings),
                db,
                power,
                spotify,
                monitor,
                notes: NoteStore::new(&data_dir)?,
                settings,
                data_dir,
                now: 0.0,
                settings_version: 0,
                toast: None,
            },
            spotify_view: SpotifyView::default(),
            clipboard_view: ClipboardView::default(),
            notes_view: NotesView::default(),
            calculator_view: CalculatorView::default(),
            launcher_view,
            monitor_view: MonitorView::default(),
            minecraft_view: MinecraftView::default(),
            games_view: GamesView::default(),
            clock_view,
            system_view: SystemView::default(),
        })
    }

    /// Wendet Design-Einstellungen an und folgt dem Hell-/Dunkelmodus.
    fn sync_theme(&mut self, ctx: &egui::Context) {
        let settings = &self.shared.settings;
        let version = self.shared.settings_version;
        if self.applied_version != Some(version) {
            self.applied_version = Some(version);
            crate::i18n::set_english(settings.language == Language::English);
            ctx.set_theme(match settings.appearance {
                Appearance::System => egui::ThemePreference::System,
                Appearance::Light => egui::ThemePreference::Light,
                Appearance::Dark => egui::ThemePreference::Dark,
            });
            ctx.set_zoom_factor(settings.zoom.clamp(0.75, 1.5));
        }

        let system_dark = ctx.theme() == egui::Theme::Dark;
        let key = (system_dark, version);
        if self.styled_for != Some(key) {
            self.shared.palette = Palette::from_settings(system_dark, settings);
            theme::apply(ctx, &self.shared.palette, settings.compact);
            self.styled_for = Some(key);
        }
    }

    /// Schließen versteckt das Fenster nur, wenn ein Tray da ist und es so eingestellt ist.
    fn hides_on_close(&self) -> bool {
        self.tray.is_some() && self.shared.settings.close_to_tray
    }

    fn visible_tabs(&self) -> Vec<Tab> {
        ordered_tabs(&self.shared.settings)
            .into_iter()
            .filter(|tab| !is_tab_hidden(&self.shared.settings, *tab))
            .collect()
    }

    fn select_tab(&mut self, tab: Tab) {
        if self.tab != tab {
            self.tab = tab;
            // Für „zuletzt verwendetes Modul“ merken (kein Neuanwenden des Designs nötig).
            self.shared.settings.last_tab = tab.key().to_owned();
            let _ = self.shared.settings.save(&self.shared.db);
        }
    }

    /// Beenden über das Tray, Schließen = Verstecken, Tray-Häkchen aktuell halten.
    fn handle_lifecycle(&mut self, ctx: &egui::Context) {
        if self.quit.load(Ordering::Relaxed) {
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }
        if ctx.input(|i| i.viewport().close_requested()) && self.hides_on_close() {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.window.hide(ctx);
        }
        if let Some(tray) = &self.tray {
            let active = self.shared.power.is_active();
            if active != self.tray_keep_awake {
                tray.keep_awake.set_checked(active);
                self.tray_keep_awake = active;
            }
        }
        // Das aktive Werkzeug wurde ausgeblendet → zum ersten sichtbaren wechseln.
        if is_tab_hidden(&self.shared.settings, self.tab) {
            if let Some(first) = self.visible_tabs().first().copied() {
                self.select_tab(first);
            }
        }
    }

    fn close_or_hide(&self, ctx: &egui::Context) {
        if self.hides_on_close() {
            self.window.hide(ctx);
        } else {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    /// Titelleiste über die ganze Breite: verschieben, minimieren, maximieren, schließen.
    fn title_bar(&mut self, ctx: &egui::Context) {
        let p = self.shared.palette;
        TopBottomPanel::top("title_bar")
            .exact_height(TITLE_BAR_HEIGHT)
            .show_separator_line(false)
            .frame(Frame::none().fill(p.sidebar_bg))
            .show(ctx, |ui| {
                let rect = ui.max_rect();
                drag_region(ui, rect, "title_bar_drag");
                ui.painter().text(
                    egui::pos2(rect.left() + 14.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    "Toolless",
                    egui::FontId::proportional(12.5),
                    p.text_secondary,
                );

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    if window_button(ui, &p, WindowButton::Close) {
                        self.close_or_hide(ctx);
                    }
                    if window_button(ui, &p, WindowButton::Maximize) {
                        toggle_maximized(ctx);
                    }
                    if window_button(ui, &p, WindowButton::Minimize) {
                        ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
                    }
                });
            });
    }

    fn sidebar(&mut self, ctx: &egui::Context) {
        let p = self.shared.palette;
        let tabs = self.visible_tabs();
        let mut selected = None;
        let mut action = None;

        SidePanel::left("sidebar")
            .exact_width(196.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(Frame::none().fill(p.sidebar_bg).inner_margin(Margin {
                left: 8.0,
                right: 8.0,
                top: 4.0,
                bottom: 8.0,
            }))
            .show(ctx, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for (index, tab) in tabs.iter().copied().enumerate() {
                    let row = theme::nav_item(ui, &p, tab.label(), self.tab == tab);
                    if row.clicked() {
                        selected = Some(tab);
                    }
                    // Ziehen und Ablegen: Werkzeug an die Stelle eines anderen setzen.
                    row.dnd_set_drag_payload(TabDrag(tab));
                    if row
                        .dnd_hover_payload::<TabDrag>()
                        .is_some_and(|dragged| dragged.0 != tab)
                    {
                        theme::drop_highlight(ui, &p, row.rect);
                    }
                    if let Some(dragged) = row.dnd_release_payload::<TabDrag>() {
                        if dragged.0 != tab {
                            action = Some(SidebarAction::MoveTo(dragged.0, tab));
                        }
                    }
                    // Rechtsklick: Seitenleiste anpassen.
                    row.context_menu(|ui| {
                        if ui
                            .add_enabled(index > 0, egui::Button::new(t("Nach oben")))
                            .clicked()
                        {
                            action = Some(SidebarAction::Move(tab, -1));
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(index + 1 < tabs.len(), egui::Button::new(t("Nach unten")))
                            .clicked()
                        {
                            action = Some(SidebarAction::Move(tab, 1));
                            ui.close_menu();
                        }
                        if tab.can_hide() {
                            ui.separator();
                            if ui.button(t("Ausblenden")).clicked() {
                                action = Some(SidebarAction::Hide(tab));
                                ui.close_menu();
                            }
                        }
                    });
                }
            });

        if let Some(tab) = selected {
            self.select_tab(tab);
        }
        match action {
            Some(SidebarAction::Move(tab, direction)) => {
                move_tab(&mut self.shared.settings, tab, direction);
                self.shared.save_settings();
            }
            Some(SidebarAction::MoveTo(tab, target)) => {
                move_tab_to(&mut self.shared.settings, tab, target);
                self.shared.save_settings();
            }
            Some(SidebarAction::Hide(tab)) => {
                set_tab_hidden(&mut self.shared.settings, tab, true);
                self.shared.save_settings();
                self.shared.notify(crate::i18n::td(format!(
                    "„{}“ ausgeblendet – unter Einstellungen wieder einblendbar",
                    tab.label()
                )));
            }
            None => {}
        }
    }

    fn content(&mut self, ctx: &egui::Context) {
        let p = self.shared.palette;
        let frame = Frame::none().fill(p.content_bg).inner_margin(Margin {
            left: 24.0,
            right: 24.0,
            top: 14.0,
            bottom: 0.0,
        });

        CentralPanel::default().frame(frame).show(ctx, |ui| {
            ui.label(RichText::new(self.tab.label()).size(22.0).strong());
            ui.add_space(6.0);

            if self.tab == Tab::Notes {
                // Die Notizansicht teilt ihre Fläche selbst auf und scrollt intern.
                self.notes_view.ui(ui, &mut self.shared);
                return;
            }
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    match self.tab {
                        Tab::Spotify => self.spotify_view.ui(ui, &mut self.shared),
                        Tab::Clipboard => self.clipboard_view.ui(ui, &mut self.shared),
                        Tab::Calculator => self.calculator_view.ui(ui, &mut self.shared),
                        Tab::Launcher => self.launcher_view.ui(ui, &mut self.shared),
                        Tab::Monitor => self.monitor_view.ui(ui, &mut self.shared),
                        Tab::Minecraft => self.minecraft_view.ui(ui, &mut self.shared),
                        Tab::Games => self.games_view.ui(ui, &mut self.shared),
                        Tab::Clock => self.clock_view.ui(ui, &mut self.shared),
                        Tab::System => self.system_view.ui(ui, &mut self.shared),
                        Tab::Notes => {}
                    }
                    // Luft unter dem letzten Element, damit nichts am Fensterrand klebt.
                    ui.add_space(24.0);
                });
        });
    }

    /// Meldung unten rechts, im Stil einer Windows-Benachrichtigung.
    fn toast(&mut self, ctx: &egui::Context) {
        let Some((message, until)) = &self.shared.toast else {
            return;
        };
        if self.shared.now > *until {
            self.shared.toast = None;
            return;
        }
        let p = self.shared.palette;
        egui::Area::new(Id::new("toast"))
            .order(Order::Foreground)
            .anchor(Align2::RIGHT_BOTTOM, egui::vec2(-18.0, -18.0))
            .interactable(false)
            .show(ctx, |ui| {
                Frame::none()
                    .fill(p.card_bg)
                    .stroke(Stroke::new(1.0_f32, p.separator))
                    .rounding(p.radius)
                    .inner_margin(Margin::symmetric(14.0, 10.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let (dot, _) =
                                ui.allocate_exact_size(egui::vec2(8.0, 8.0), Sense::hover());
                            ui.painter().circle_filled(dot.center(), 4.0, p.accent);
                            // Einzeilig halten – sonst bricht der Text in der schmalen Fläche um.
                            let mut text: String = message.chars().take(110).collect();
                            if message.chars().count() > 110 {
                                text.push('…');
                            }
                            ui.add(egui::Label::new(RichText::new(text).color(p.text)).extend());
                        });
                    });
            });
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    /// Begrenzt die Bildrate auf 60 Bilder pro Sekunde.
    fn limit_frame_rate(&mut self) {
        let elapsed = self.last_frame.elapsed();
        if elapsed < MIN_FRAME_TIME {
            thread::sleep(MIN_FRAME_TIME - elapsed);
        }
        self.last_frame = Instant::now();
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.shared.now = ctx.input(|i| i.time);
        self.sync_theme(ctx);
        self.handle_lifecycle(ctx);
        self.shared
            .spotify
            .set_polling(self.tab == Tab::Spotify && self.window.is_visible());
        self.shared
            .monitor
            .set_active(self.tab == Tab::Monitor && self.window.is_visible());

        // Wecker und Vollbildfläche laufen unabhängig vom offenen Werkzeug.
        if self.clock_view.background(ctx, &mut self.shared) {
            self.window.show(ctx);
            self.select_tab(Tab::Clock);
        }

        self.title_bar(ctx);
        self.sidebar(ctx);
        self.content(ctx);

        // Feine Kontur um das Fenster – nicht im Vollbild, dort gibt es keinen Rand.
        let fills_screen = ctx.input(|i| {
            let viewport = i.viewport();
            viewport.maximized.unwrap_or(false) || viewport.fullscreen.unwrap_or(false)
        });
        if !fills_screen {
            ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("window_border")))
                .rect_stroke(
                    ctx.screen_rect().shrink(0.5),
                    8.0,
                    Stroke::new(1.0_f32, self.shared.palette.separator),
                );
        }

        resize_edges(ctx);
        self.toast(ctx);
        self.limit_frame_rate();
    }
}

// ───────── Fensterrahmen ─────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum WindowButton {
    Minimize,
    Maximize,
    Close,
}

/// Fensterknopf im Windows-Stil (klein, wie bei Discord). Liefert `true` bei Klick.
fn window_button(ui: &mut Ui, p: &Palette, kind: WindowButton) -> bool {
    let size = egui::vec2(38.0, ui.available_height());
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hovered = response.hovered();

    let (background, foreground) = match (kind, hovered) {
        (WindowButton::Close, true) => (Some(Color32::from_rgb(232, 17, 35)), Color32::WHITE),
        (_, true) => (Some(p.control_hover), p.text),
        _ => (None, p.text_secondary),
    };
    if let Some(background) = background {
        ui.painter().rect_filled(rect, 0.0, background);
    }

    let center = rect.center();
    let half = 4.5;
    let stroke = Stroke::new(1.0_f32, foreground);
    let painter = ui.painter();
    match kind {
        WindowButton::Minimize => {
            painter.line_segment(
                [
                    egui::pos2(center.x - half, center.y),
                    egui::pos2(center.x + half, center.y),
                ],
                stroke,
            );
        }
        WindowButton::Maximize => {
            painter.rect_stroke(
                Rect::from_center_size(center, egui::vec2(2.0 * half, 2.0 * half)),
                1.0,
                stroke,
            );
        }
        WindowButton::Close => {
            painter.line_segment(
                [
                    egui::pos2(center.x - half, center.y - half),
                    egui::pos2(center.x + half, center.y + half),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    egui::pos2(center.x - half, center.y + half),
                    egui::pos2(center.x + half, center.y - half),
                ],
                stroke,
            );
        }
    }
    response.clicked()
}

fn toggle_maximized(ctx: &egui::Context) {
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
}

/// Fläche, an der sich das rahmenlose Fenster verschieben lässt (Doppelklick = maximieren).
fn drag_region(ui: &mut Ui, rect: Rect, id_salt: &str) {
    let response = ui.interact(rect, ui.id().with(id_salt), Sense::click_and_drag());
    if response.double_clicked() {
        toggle_maximized(ui.ctx());
    }
    if response.drag_started_by(PointerButton::Primary) {
        ui.ctx().send_viewport_cmd(ViewportCommand::StartDrag);
    }
}

/// Größenänderung an den Fensterkanten (ein rahmenloses Fenster hat keine eigenen Griffe).
fn resize_edges(ctx: &egui::Context) {
    if ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
        return;
    }
    let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else {
        return;
    };
    let rect = ctx.screen_rect();
    let left = pos.x <= rect.left() + RESIZE_MARGIN;
    let right = pos.x >= rect.right() - RESIZE_MARGIN;
    let top = pos.y <= rect.top() + RESIZE_MARGIN;
    let bottom = pos.y >= rect.bottom() - RESIZE_MARGIN;

    let (direction, cursor) = match (left, right, top, bottom) {
        (true, _, true, _) => (ResizeDirection::NorthWest, CursorIcon::ResizeNwSe),
        (_, true, true, _) => (ResizeDirection::NorthEast, CursorIcon::ResizeNeSw),
        (true, _, _, true) => (ResizeDirection::SouthWest, CursorIcon::ResizeNeSw),
        (_, true, _, true) => (ResizeDirection::SouthEast, CursorIcon::ResizeNwSe),
        (true, ..) => (ResizeDirection::West, CursorIcon::ResizeHorizontal),
        (_, true, ..) => (ResizeDirection::East, CursorIcon::ResizeHorizontal),
        (_, _, true, _) => (ResizeDirection::North, CursorIcon::ResizeVertical),
        (_, _, _, true) => (ResizeDirection::South, CursorIcon::ResizeVertical),
        _ => return,
    };
    ctx.set_cursor_icon(cursor);
    if ctx.input(|i| i.pointer.primary_pressed()) {
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
    }
}
