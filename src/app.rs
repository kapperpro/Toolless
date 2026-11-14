//! Hauptfenster: eigener Fensterrahmen im macOS-Stil, Seitenleiste, Modulwechsel,
//! Hintergrundbetrieb (Schließen = in den Tray verstecken).

use std::{
    fmt::Display,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use eframe::egui::{
    self, viewport::ResizeDirection, Align, Align2, CentralPanel, Color32, CursorIcon, Frame, Id,
    LayerId, Layout, Margin, Order, PointerButton, Rect, RichText, Rounding, ScrollArea, Sense,
    SidePanel, Stroke, TopBottomPanel, Ui, ViewportCommand,
};

use crate::{
    db::Db,
    error::{AppError, AppResult},
    platform::NativeWindow,
    services::{notes::NoteStore, power::PowerState, spotify::SpotifyHandle},
    theme::{self, Palette, WINDOW_RADIUS},
    tray::{self, Tray, TrayDeps},
    ui::{
        calculator::CalculatorView, clipboard::ClipboardView, launcher::LauncherView,
        notes::NotesView, spotify::SpotifyView, system::SystemView,
    },
};

const TOAST_SECONDS: f64 = 3.8;
/// Breite des unsichtbaren Rands, an dem sich das Fenster in der Größe ziehen lässt.
const RESIZE_MARGIN: f32 = 5.0;

/// Zustand und Dienste, die allen Ansichten zur Verfügung stehen.
pub struct Shared {
    pub palette: Palette,
    pub db: Arc<Db>,
    pub power: Arc<PowerState>,
    pub spotify: SpotifyHandle,
    pub notes: NoteStore,
    /// Laufzeit in Sekunden (egui-Uhr), pro Frame aktualisiert.
    pub now: f64,
    toast: Option<(String, f64)>,
}

impl Shared {
    /// Zeigt eine kurze Meldung am unteren Fensterrand.
    pub fn notify(&mut self, message: impl Display) {
        self.toast = Some((message.to_string(), self.now + TOAST_SECONDS));
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Spotify,
    Clipboard,
    Notes,
    Calculator,
    Launcher,
    System,
}

impl Tab {
    const ALL: [Tab; 6] = [
        Tab::Spotify,
        Tab::Clipboard,
        Tab::Notes,
        Tab::Calculator,
        Tab::Launcher,
        Tab::System,
    ];

    fn label(self) -> &'static str {
        match self {
            Tab::Spotify => "Spotify",
            Tab::Clipboard => "Zwischenablage",
            Tab::Notes => "Notizen",
            Tab::Calculator => "Rechner",
            Tab::Launcher => "App-Starter",
            Tab::System => "System",
        }
    }
}

pub struct App {
    tab: Tab,
    /// Für welches Farbschema der Stil zuletzt gesetzt wurde.
    styled_dark: Option<bool>,
    window: NativeWindow,
    tray: Option<Tray>,
    tray_keep_awake: bool,
    quit: Arc<AtomicBool>,
    shared: Shared,
    spotify_view: SpotifyView,
    clipboard_view: ClipboardView,
    notes_view: NotesView,
    calculator_view: CalculatorView,
    launcher_view: LauncherView,
    system_view: SystemView,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, visible: bool) -> AppResult<Self> {
        let data_dir = dirs::data_dir()
            .ok_or_else(|| AppError::other("Datenordner des Systems nicht gefunden"))?
            .join("Toolless");
        std::fs::create_dir_all(&data_dir)?;

        let db = Arc::new(Db::open(&data_dir.join("toolless.sqlite"))?);
        let power = Arc::new(PowerState::default());
        let window = NativeWindow::new(cc, visible);
        let quit = Arc::new(AtomicBool::new(false));
        let spotify = SpotifyHandle::spawn(
            Arc::clone(&db),
            data_dir.join("spotify_token.json"),
            cc.egui_ctx.clone(),
        );

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

        Ok(Self {
            tab: Tab::Spotify,
            styled_dark: None,
            window,
            tray,
            tray_keep_awake: false,
            quit,
            shared: Shared {
                palette: Palette::new(false),
                db,
                power,
                spotify,
                notes: NoteStore::new(&data_dir)?,
                now: 0.0,
                toast: None,
            },
            spotify_view: SpotifyView::default(),
            clipboard_view: ClipboardView::default(),
            notes_view: NotesView::default(),
            calculator_view: CalculatorView::default(),
            launcher_view: LauncherView::default(),
            system_view: SystemView::default(),
        })
    }

    /// Folgt dem Hell-/Dunkelmodus des Systems.
    fn sync_theme(&mut self, ctx: &egui::Context) {
        let dark = ctx.style().visuals.dark_mode;
        if self.styled_dark != Some(dark) {
            self.shared.palette = Palette::new(dark);
            theme::apply(ctx, &self.shared.palette);
            self.styled_dark = Some(dark);
        }
    }

    /// Beenden über das Tray, Schließen = Verstecken, Tray-Häkchen aktuell halten.
    fn handle_lifecycle(&mut self, ctx: &egui::Context) {
        if self.quit.load(Ordering::Relaxed) {
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }
        if ctx.input(|i| i.viewport().close_requested()) && self.tray.is_some() {
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
    }

    fn close_or_hide(&self, ctx: &egui::Context) {
        if self.tray.is_some() {
            self.window.hide(ctx);
        } else {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }

    fn sidebar(&mut self, ctx: &egui::Context) {
        let p = self.shared.palette;
        let frame = Frame::none()
            .fill(p.sidebar_bg)
            .rounding(Rounding {
                nw: WINDOW_RADIUS,
                sw: WINDOW_RADIUS,
                ne: 0.0,
                se: 0.0,
            })
            .inner_margin(Margin::same(10.0));

        SidePanel::left("sidebar")
            .exact_width(200.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(frame)
            .show(ctx, |ui| {
                // Kopfbereich: Fenster verschieben + „Ampel“-Buttons.
                let (title_rect, _) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), Sense::hover());
                drag_region(ui, title_rect.expand(10.0), "sidebar_drag");
                match traffic_lights(ui, title_rect) {
                    Some(Light::Close) => self.close_or_hide(ctx),
                    Some(Light::Minimize) => {
                        ctx.send_viewport_cmd(ViewportCommand::Minimized(true))
                    }
                    Some(Light::Zoom) => toggle_maximized(ctx),
                    None => {}
                }

                ui.add_space(4.0);
                ui.label(theme::secondary(&p, "  Werkzeuge").size(11.5).strong());
                ui.spacing_mut().item_spacing.y = 2.0;
                for tab in Tab::ALL {
                    if theme::nav_item(ui, &p, tab.label(), self.tab == tab).clicked() {
                        self.tab = tab;
                    }
                }

                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    ui.label(
                        theme::secondary(
                            &p,
                            "Toolless läuft im Tray weiter, wenn du das Fenster schließt.",
                        )
                        .size(11.5),
                    );
                });
            });
    }

    fn toolbar(&mut self, ctx: &egui::Context) {
        let p = self.shared.palette;
        let frame = Frame::none()
            .fill(p.content_bg)
            .rounding(Rounding {
                ne: WINDOW_RADIUS,
                nw: 0.0,
                sw: 0.0,
                se: 0.0,
            })
            .inner_margin(Margin::symmetric(24.0, 0.0));

        TopBottomPanel::top("toolbar")
            .exact_height(48.0)
            .show_separator_line(false)
            .frame(frame)
            .show(ctx, |ui| {
                let rect = ui.max_rect().expand2(egui::vec2(24.0, 0.0));
                drag_region(ui, rect, "toolbar_drag");
                ui.painter()
                    .hline(rect.x_range(), rect.bottom(), Stroke::new(1.0, p.separator));
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.label(RichText::new(self.tab.label()).size(16.0).strong());
                });
            });
    }

    fn content(&mut self, ctx: &egui::Context) {
        let p = self.shared.palette;
        let frame = Frame::none()
            .fill(p.content_bg)
            .rounding(Rounding {
                se: WINDOW_RADIUS,
                nw: 0.0,
                ne: 0.0,
                sw: 0.0,
            })
            .inner_margin(Margin::same(20.0));

        CentralPanel::default().frame(frame).show(ctx, |ui| {
            if self.tab == Tab::Notes {
                // Die Notizansicht teilt ihre Fläche selbst auf und scrollt intern.
                self.notes_view.ui(ui, &mut self.shared);
                return;
            }
            ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| match self.tab {
                    Tab::Spotify => self.spotify_view.ui(ui, &mut self.shared),
                    Tab::Clipboard => self.clipboard_view.ui(ui, &mut self.shared),
                    Tab::Calculator => self.calculator_view.ui(ui, &mut self.shared),
                    Tab::Launcher => self.launcher_view.ui(ui, &mut self.shared),
                    Tab::System => self.system_view.ui(ui, &mut self.shared),
                    Tab::Notes => {}
                });
        });
    }

    fn toast(&mut self, ctx: &egui::Context) {
        let Some((message, until)) = &self.shared.toast else {
            return;
        };
        if self.shared.now > *until {
            self.shared.toast = None;
            return;
        }
        egui::Area::new(Id::new("toast"))
            .order(Order::Foreground)
            .anchor(Align2::CENTER_BOTTOM, egui::vec2(0.0, -22.0))
            .interactable(false)
            .show(ctx, |ui| {
                Frame::none()
                    .fill(Color32::from_rgba_unmultiplied(50, 50, 54, 240))
                    .rounding(10.0)
                    .inner_margin(Margin::symmetric(14.0, 8.0))
                    .show(ui, |ui| {
                        ui.label(RichText::new(message).color(Color32::WHITE));
                    });
            });
        ctx.request_repaint_after(Duration::from_millis(250));
    }
}

impl eframe::App for App {
    /// Transparenter Hintergrund – die abgerundeten Fensterecken zeichnen wir selbst.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::TRANSPARENT.to_array()
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.shared.now = ctx.input(|i| i.time);
        self.sync_theme(ctx);
        self.handle_lifecycle(ctx);
        self.shared
            .spotify
            .set_polling(self.tab == Tab::Spotify && self.window.is_visible());

        self.sidebar(ctx);
        self.toolbar(ctx);
        self.content(ctx);

        // Feine Kontur um das gesamte Fenster.
        ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("window_border")))
            .rect_stroke(
                ctx.screen_rect().shrink(0.5),
                WINDOW_RADIUS,
                Stroke::new(1.0, self.shared.palette.separator),
            );

        resize_edges(ctx);
        self.toast(ctx);
    }
}

// ───────── Fensterrahmen ─────────

#[derive(Clone, Copy)]
enum Light {
    Close,
    Minimize,
    Zoom,
}

/// Die drei „Ampel“-Buttons oben links. Liefert den angeklickten Button.
fn traffic_lights(ui: &mut Ui, area: Rect) -> Option<Light> {
    const LIGHTS: [(Light, Color32, &str); 3] = [
        (
            Light::Close,
            Color32::from_rgb(255, 95, 87),
            "Schließen (läuft im Tray weiter)",
        ),
        (
            Light::Minimize,
            Color32::from_rgb(254, 188, 46),
            "Minimieren",
        ),
        (Light::Zoom, Color32::from_rgb(40, 200, 64), "Zoomen"),
    ];

    let mut clicked = None;
    for (index, (light, color, tooltip)) in LIGHTS.into_iter().enumerate() {
        let center = egui::pos2(area.left() + 12.0 + index as f32 * 20.0, area.center().y);
        let rect = Rect::from_center_size(center, egui::vec2(16.0, 16.0));
        let response = ui
            .interact(rect, ui.id().with(("traffic_light", index)), Sense::click())
            .on_hover_text(tooltip);
        let fill = if response.hovered() {
            color.gamma_multiply(0.8)
        } else {
            color
        };
        ui.painter().circle_filled(center, 6.0, fill);
        if response.clicked() {
            clicked = Some(light);
        }
    }
    clicked
}

fn toggle_maximized(ctx: &egui::Context) {
    let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
    ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
}

/// Fläche, an der sich das rahmenlose Fenster verschieben lässt (Doppelklick = zoomen).
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
