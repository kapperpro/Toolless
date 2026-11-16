//! Aussehen: an Windows 11 angelehnte Farben und Bausteine. Fast alles ist über
//! die Einstellungen anpassbar (Farben, Eckenradius, Dichte).

use eframe::egui::{
    self, Align, Button, Color32, FontId, Frame, Layout, Margin, Rect, Response, RichText,
    Rounding, Sense, Stroke, TextStyle, TextureHandle, Ui, UiBuilder, Vec2,
};

use crate::settings::Settings;

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub accent: Color32,
    /// Textfarbe auf Flächen in der Akzentfarbe.
    pub on_accent: Color32,
    pub danger: Color32,
    pub success: Color32,
    pub text: Color32,
    pub text_secondary: Color32,
    pub sidebar_bg: Color32,
    pub content_bg: Color32,
    pub card_bg: Color32,
    pub field_bg: Color32,
    pub control_bg: Color32,
    pub control_hover: Color32,
    pub selection: Color32,
    pub separator: Color32,
    /// Eckenradius von Bedienelementen bzw. Karten.
    pub radius: f32,
    pub card_radius: f32,
}

fn rgb(color: [u8; 3]) -> Color32 {
    Color32::from_rgb(color[0], color[1], color[2])
}

pub fn to_rgb(color: Color32) -> [u8; 3] {
    [color.r(), color.g(), color.b()]
}

/// Wahrgenommene Helligkeit (0 = schwarz, 1 = weiß).
fn luminance(color: Color32) -> f32 {
    (0.299 * color.r() as f32 + 0.587 * color.g() as f32 + 0.114 * color.b() as f32) / 255.0
}

/// Hellt eine Farbe auf (positiv) oder dunkelt sie ab (negativ).
fn shift(color: Color32, amount: i16) -> Color32 {
    let channel = |value: u8| (value as i16 + amount).clamp(0, 255) as u8;
    Color32::from_rgb(channel(color.r()), channel(color.g()), channel(color.b()))
}

impl Palette {
    /// Baut die Palette aus den Einstellungen. `system_dark`: Das aktive Design ist dunkel.
    pub fn from_settings(system_dark: bool, settings: &Settings) -> Self {
        // Alle Flächenfarben leiten sich vom Hintergrund ab – so passt auch ein
        // frei gewählter Hintergrund automatisch zu Karten, Feldern und Text.
        let content_bg = settings
            .custom_background
            .map(rgb)
            .unwrap_or(if system_dark {
                Color32::from_rgb(32, 32, 32)
            } else {
                Color32::from_rgb(243, 243, 243)
            });
        let dark = luminance(content_bg) < 0.5;
        // Im dunklen Design werden abgeleitete Flächen heller, im hellen dunkler.
        let step = |amount: i16| shift(content_bg, if dark { amount } else { -amount });

        let accent = settings
            .custom_accent
            .map(rgb)
            .unwrap_or_else(|| settings.accent.color(dark));
        let on_accent = if luminance(accent) > 0.6 {
            Color32::from_gray(16)
        } else {
            Color32::WHITE
        };
        let radius = settings.rounding.clamp(0.0, 14.0);

        Self {
            dark,
            accent,
            on_accent,
            danger: if dark {
                Color32::from_rgb(255, 110, 100)
            } else {
                Color32::from_rgb(196, 43, 28)
            },
            success: if dark {
                Color32::from_rgb(108, 203, 95)
            } else {
                Color32::from_rgb(15, 123, 15)
            },
            text: if dark {
                Color32::from_gray(240)
            } else {
                Color32::from_gray(26)
            },
            text_secondary: if dark {
                Color32::from_gray(165)
            } else {
                Color32::from_gray(96)
            },
            sidebar_bg: settings.custom_sidebar.map(rgb).unwrap_or(step(-5)),
            content_bg,
            card_bg: settings.custom_card.map(rgb).unwrap_or(if dark {
                step(11)
            } else {
                shift(content_bg, 9)
            }),
            field_bg: if dark {
                step(-8)
            } else {
                shift(content_bg, 12)
            },
            control_bg: step(if dark { 26 } else { 14 }),
            control_hover: step(if dark { 38 } else { 24 }),
            selection: step(if dark { 24 } else { 16 }),
            separator: step(if dark { 30 } else { 22 }),
            radius,
            card_radius: (radius * 2.0).min(12.0),
        }
    }
}

/// Überträgt die Palette auf den globalen egui-Stil.
pub fn apply(ctx: &egui::Context, p: &Palette, compact: bool) {
    let mut style = (*ctx.style()).clone();

    if compact {
        style.spacing.item_spacing = Vec2::new(6.0, 5.0);
        style.spacing.button_padding = Vec2::new(9.0, 3.0);
        style.spacing.interact_size.y = 24.0;
    } else {
        style.spacing.item_spacing = Vec2::new(8.0, 8.0);
        style.spacing.button_padding = Vec2::new(12.0, 5.0);
        style.spacing.interact_size.y = 28.0;
    }

    let sizes = [
        (TextStyle::Body, FontId::proportional(14.0)),
        (TextStyle::Button, FontId::proportional(14.0)),
        (TextStyle::Heading, FontId::proportional(22.0)),
        (TextStyle::Small, FontId::proportional(12.0)),
        (TextStyle::Monospace, FontId::monospace(13.0)),
    ];
    for (text_style, font) in sizes {
        style.text_styles.insert(text_style, font);
    }

    // Text in der Oberfläche ist Beschriftung, kein markierbarer Fließtext –
    // Klicks auf Zeilen sollen bei der Zeile ankommen.
    style.interaction.selectable_labels = false;

    let mut visuals = if p.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.panel_fill = p.content_bg;
    visuals.window_fill = p.card_bg;
    visuals.window_stroke = Stroke::new(1.0_f32, p.separator);
    visuals.window_rounding = Rounding::same(p.card_radius);
    visuals.menu_rounding = Rounding::same(p.card_radius.min(8.0));
    visuals.extreme_bg_color = p.field_bg;
    visuals.faint_bg_color = p.card_bg;
    visuals.hyperlink_color = p.accent;
    visuals.selection.bg_fill = p.accent.gamma_multiply(0.45);
    visuals.selection.stroke = Stroke::new(1.0_f32, p.accent);

    let rounding = Rounding::same(p.radius);
    let widgets = &mut visuals.widgets;
    widgets.noninteractive.rounding = rounding;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.separator);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.text);

    widgets.inactive.rounding = rounding;
    widgets.inactive.bg_fill = p.control_bg;
    widgets.inactive.weak_bg_fill = p.control_bg;
    widgets.inactive.bg_stroke = Stroke::NONE;
    widgets.inactive.fg_stroke = Stroke::new(1.0_f32, p.text);

    widgets.hovered.rounding = rounding;
    widgets.hovered.bg_fill = p.control_hover;
    widgets.hovered.weak_bg_fill = p.control_hover;
    widgets.hovered.bg_stroke = Stroke::NONE;
    widgets.hovered.fg_stroke = Stroke::new(1.0_f32, p.text);
    widgets.hovered.expansion = 0.0;

    widgets.active.rounding = rounding;
    widgets.active.bg_fill = p.control_hover;
    widgets.active.weak_bg_fill = p.control_hover;
    widgets.active.bg_stroke = Stroke::new(1.0_f32, p.accent);
    widgets.active.fg_stroke = Stroke::new(1.0_f32, p.text);
    widgets.active.expansion = 0.0;

    widgets.open.rounding = rounding;
    widgets.open.bg_fill = p.control_hover;
    widgets.open.weak_bg_fill = p.control_hover;

    style.visuals = visuals;
    ctx.set_style(style);
}

/// Karte: leicht abgesetzte Fläche mit feinem Rand.
pub fn card(p: &Palette) -> Frame {
    Frame::none()
        .fill(p.card_bg)
        .stroke(Stroke::new(1.0_f32, p.separator.gamma_multiply(0.6)))
        .rounding(p.card_radius)
        .inner_margin(Margin::same(14.0))
}

pub fn card_title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(crate::i18n::t(text)).strong());
}

pub fn secondary(p: &Palette, text: impl Into<String>) -> RichText {
    RichText::new(crate::i18n::td(text.into())).color(p.text_secondary)
}

/// Hervorgehobener Button in der Akzentfarbe.
pub fn primary_button(ui: &mut Ui, p: &Palette, text: &str, enabled: bool) -> Response {
    let button = Button::new(RichText::new(crate::i18n::t(text)).color(p.on_accent)).fill(p.accent);
    ui.add_enabled(enabled, button)
}

/// Auswahl-Chip; aktiv = Akzentfarbe.
pub fn chip(ui: &mut Ui, p: &Palette, text: &str, active: bool) -> Response {
    let (fill, color) = if active {
        (p.accent, p.on_accent)
    } else {
        (p.control_bg, p.text)
    };
    ui.add(Button::new(RichText::new(crate::i18n::t(text)).color(color)).fill(fill))
}

/// Kleines farbiges Etikett (z. B. Kategorie).
pub fn badge(ui: &mut Ui, p: &Palette, text: &str) {
    let galley = ui.painter().layout_no_wrap(
        crate::i18n::t(text).to_owned(),
        FontId::proportional(11.5),
        p.on_accent,
    );
    let size = galley.size() + Vec2::new(12.0, 4.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(rect, size.y / 2.0, p.accent);
        ui.painter()
            .galley(rect.min + Vec2::new(6.0, 2.0), galley, p.on_accent);
    }
}

/// Schalter im Windows-Stil: aus = umrandet, an = Akzentfarbe.
pub fn toggle(ui: &mut Ui, p: &Palette, on: &mut bool) -> Response {
    let size = Vec2::new(40.0, 20.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let how_on = ui.ctx().animate_bool(response.id, *on);
        let radius = rect.height() / 2.0;
        let knob_radius = if response.hovered() { 7.0 } else { 6.0 };
        if *on {
            ui.painter().rect_filled(rect, radius, p.accent);
        } else {
            ui.painter().rect_filled(rect, radius, p.field_bg);
            ui.painter().rect_stroke(
                rect.shrink(0.5),
                radius,
                Stroke::new(1.0_f32, p.text_secondary),
            );
        }
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), how_on);
        let knob = if *on { p.on_accent } else { p.text_secondary };
        ui.painter()
            .circle_filled(egui::pos2(x, rect.center().y), knob_radius, knob);
    }
    response
}

/// Zeile einer Auswahlliste (Seitenleiste, Notizliste). Die aktive Zeile trägt
/// links einen Akzentstrich wie in Windows-Navigationsleisten.
pub fn nav_item(ui: &mut Ui, p: &Palette, label: &str, selected: bool) -> Response {
    let size = Vec2::new(ui.available_width(), 32.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    if ui.is_rect_visible(rect) {
        if selected {
            ui.painter().rect_filled(rect, p.radius, p.selection);
            let bar = Rect::from_center_size(
                egui::pos2(rect.left() + 1.5, rect.center().y),
                Vec2::new(3.0, 16.0),
            );
            ui.painter().rect_filled(bar, 1.5, p.accent);
        } else if response.hovered() {
            ui.painter()
                .rect_filled(rect, p.radius, p.selection.gamma_multiply(0.6));
        }
        let galley =
            ui.painter()
                .layout_no_wrap(label.to_owned(), FontId::proportional(14.0), p.text);
        // Zu lange Beschriftungen am rechten Rand abschneiden.
        let text_rect = rect.shrink2(Vec2::new(12.0, 0.0));
        let pos = egui::pos2(text_rect.left(), rect.center().y - galley.size().y / 2.0);
        ui.painter()
            .with_clip_rect(text_rect)
            .galley(pos, galley, p.text);
    }
    response
}

/// Einzeilige Vorschau eines (evtl. mehrzeiligen) Textes.
pub fn first_line(text: &str, max_chars: usize) -> String {
    let line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    let mut preview: String = line.chars().take(max_chars).collect();
    if line.chars().count() > max_chars || text.trim().lines().count() > 1 {
        preview.push('…');
    }
    preview
}

/// Zentrierter Hinweistext für leere Zustände.
pub fn empty_state(ui: &mut Ui, p: &Palette, text: &str) {
    ui.add_space(40.0);
    ui.vertical_centered(|ui| {
        ui.label(secondary(p, text));
    });
}

/// Anklickbare Listenzeile: Die ganze Fläche reagiert auf Klick und Rechtsklick,
/// Bedienelemente im Inhalt (Buttons, Checkboxen) behalten Vorrang.
///
/// Der Inhalt wird von rechts nach links angeordnet: zuerst Aktionen am rechten
/// Rand, danach der Text, der die Restbreite füllt.
pub fn clickable_row(
    ui: &mut Ui,
    p: &Palette,
    height: f32,
    selected: bool,
    add_contents: impl FnOnce(&mut Ui),
) -> Response {
    let size = Vec2::new(ui.available_width(), height);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    if ui.is_rect_visible(rect) {
        if selected {
            ui.painter().rect_filled(rect, p.radius, p.selection);
        } else if response.contains_pointer() {
            ui.painter()
                .rect_filled(rect, p.radius, p.selection.gamma_multiply(0.6));
        }
        // Eigenes Kind-Ui: später registrierte Widgets liegen über der Zeile.
        let mut child = ui.new_child(
            UiBuilder::new()
                .max_rect(rect.shrink2(Vec2::new(8.0, 2.0)))
                .layout(Layout::right_to_left(Align::Center)),
        );
        add_contents(&mut child);
    }
    response
}

/// Kleines App-Icon; ohne Bild ein farbiges Feld mit dem Anfangsbuchstaben.
pub fn app_icon(ui: &mut Ui, p: &Palette, texture: Option<&TextureHandle>, name: &str, side: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    match texture {
        Some(texture) => {
            let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            ui.painter().image(texture.id(), rect, uv, Color32::WHITE);
        }
        None => {
            ui.painter().rect_filled(rect, p.radius, p.control_hover);
            let letter = name
                .chars()
                .next()
                .map(|c| c.to_uppercase().to_string())
                .unwrap_or_default();
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                letter,
                FontId::proportional(side * 0.55),
                p.text,
            );
        }
    }
}

/// Markiert ein mögliches Ziel beim Ziehen und Ablegen.
pub fn drop_highlight(ui: &Ui, p: &Palette, rect: Rect) {
    ui.painter()
        .rect_stroke(rect.shrink(1.0), p.radius, Stroke::new(2.0_f32, p.accent));
}

/// Schieberegler im Windows-Stil: schmale Spur, gefüllt bis zum Griff, runder Griff
/// mit farbigem Kern. `value` wird auf `min..=max` begrenzt.
pub fn slider(
    ui: &mut Ui,
    p: &Palette,
    value: &mut f32,
    min: f32,
    max: f32,
    width: f32,
) -> Response {
    let size = Vec2::new(width, 22.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let knob_radius = 8.0;
    let track_left = rect.left() + knob_radius;
    let track_width = (rect.width() - 2.0 * knob_radius).max(1.0);

    if let Some(pointer) = response.interact_pointer_pos() {
        let fraction = ((pointer.x - track_left) / track_width).clamp(0.0, 1.0);
        let new_value = min + (max - min) * fraction;
        if (new_value - *value).abs() > f32::EPSILON {
            *value = new_value;
            response.mark_changed();
        }
    }
    *value = value.clamp(min, max);

    if ui.is_rect_visible(rect) {
        let fraction = if max > min {
            (*value - min) / (max - min)
        } else {
            0.0
        };
        let center_y = rect.center().y;
        let knob_x = track_left + track_width * fraction;
        let track = Rect::from_min_max(
            egui::pos2(track_left, center_y - 2.0),
            egui::pos2(track_left + track_width, center_y + 2.0),
        );
        ui.painter().rect_filled(track, 2.0, p.control_hover);
        let filled = Rect::from_min_max(track.min, egui::pos2(knob_x, track.max.y));
        ui.painter().rect_filled(filled, 2.0, p.accent);

        let knob = egui::pos2(knob_x, center_y);
        let active = response.hovered() || response.dragged();
        ui.painter().circle_filled(knob, knob_radius, p.card_bg);
        ui.painter()
            .circle_stroke(knob, knob_radius, Stroke::new(1.0_f32, p.separator));
        ui.painter()
            .circle_filled(knob, if active { 5.5 } else { 4.5 }, p.accent);
    }
    response
}
