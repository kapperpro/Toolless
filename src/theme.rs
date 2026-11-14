//! Aussehen: an macOS angelehnte Farben, Abstände und wiederverwendbare Bausteine.

use eframe::egui::{
    self, Button, Color32, FontId, Frame, Margin, Response, RichText, Rounding, Sense, Stroke,
    TextStyle, Ui, Vec2,
};

pub const WINDOW_RADIUS: f32 = 10.0;
pub const CARD_RADIUS: f32 = 12.0;
pub const CONTROL_RADIUS: f32 = 7.0;

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub accent: Color32,
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
}

impl Palette {
    pub fn new(dark: bool) -> Self {
        if dark {
            Self {
                dark,
                accent: Color32::from_rgb(10, 132, 255),
                danger: Color32::from_rgb(255, 69, 58),
                success: Color32::from_rgb(48, 209, 88),
                text: Color32::from_gray(235),
                text_secondary: Color32::from_gray(152),
                sidebar_bg: Color32::from_rgb(44, 44, 48),
                content_bg: Color32::from_rgb(30, 30, 32),
                card_bg: Color32::from_rgb(42, 42, 45),
                field_bg: Color32::from_rgb(24, 24, 26),
                control_bg: Color32::from_gray(62),
                control_hover: Color32::from_gray(78),
                selection: Color32::from_gray(66),
                separator: Color32::from_gray(58),
            }
        } else {
            Self {
                dark,
                accent: Color32::from_rgb(0, 122, 255),
                danger: Color32::from_rgb(255, 59, 48),
                success: Color32::from_rgb(52, 199, 89),
                text: Color32::from_gray(28),
                text_secondary: Color32::from_gray(112),
                sidebar_bg: Color32::from_rgb(233, 233, 238),
                content_bg: Color32::WHITE,
                card_bg: Color32::from_rgb(245, 245, 247),
                field_bg: Color32::WHITE,
                control_bg: Color32::from_gray(228),
                control_hover: Color32::from_gray(214),
                selection: Color32::from_gray(212),
                separator: Color32::from_gray(216),
            }
        }
    }
}

/// Überträgt die Palette auf den globalen egui-Stil.
pub fn apply(ctx: &egui::Context, p: &Palette) {
    let mut style = (*ctx.style()).clone();

    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(12.0, 5.0);
    style.spacing.interact_size.y = 28.0;

    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(14.0));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(14.0));
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::proportional(22.0));
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(12.0));
    style
        .text_styles
        .insert(TextStyle::Monospace, FontId::monospace(13.0));

    let mut visuals = if p.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.panel_fill = p.content_bg;
    visuals.window_fill = p.card_bg;
    visuals.extreme_bg_color = p.field_bg;
    visuals.faint_bg_color = p.card_bg;
    visuals.hyperlink_color = p.accent;
    visuals.selection.bg_fill = p.accent.gamma_multiply(0.45);
    visuals.selection.stroke = Stroke::new(1.0, p.accent);

    let rounding = Rounding::same(CONTROL_RADIUS);
    let widgets = &mut visuals.widgets;
    widgets.noninteractive.rounding = rounding;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.separator);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);

    widgets.inactive.rounding = rounding;
    widgets.inactive.bg_fill = p.control_bg;
    widgets.inactive.weak_bg_fill = p.control_bg;
    widgets.inactive.bg_stroke = Stroke::NONE;
    widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);

    widgets.hovered.rounding = rounding;
    widgets.hovered.bg_fill = p.control_hover;
    widgets.hovered.weak_bg_fill = p.control_hover;
    widgets.hovered.bg_stroke = Stroke::NONE;
    widgets.hovered.fg_stroke = Stroke::new(1.0, p.text);
    widgets.hovered.expansion = 0.0;

    widgets.active.rounding = rounding;
    widgets.active.bg_fill = p.control_hover;
    widgets.active.weak_bg_fill = p.control_hover;
    widgets.active.bg_stroke = Stroke::new(1.0, p.accent);
    widgets.active.fg_stroke = Stroke::new(1.0, p.text);
    widgets.active.expansion = 0.0;

    widgets.open.rounding = rounding;

    style.visuals = visuals;
    ctx.set_style(style);
}

/// Karte im Stil gruppierter macOS-Listen.
pub fn card(p: &Palette) -> Frame {
    Frame::none()
        .fill(p.card_bg)
        .rounding(CARD_RADIUS)
        .inner_margin(Margin::same(16.0))
}

pub fn card_title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).strong());
}

pub fn secondary(p: &Palette, text: impl Into<String>) -> RichText {
    RichText::new(text).color(p.text_secondary)
}

/// Hervorgehobener Button in der Akzentfarbe.
pub fn primary_button(ui: &mut Ui, p: &Palette, text: &str, enabled: bool) -> Response {
    let button = Button::new(RichText::new(text).color(Color32::WHITE)).fill(p.accent);
    ui.add_enabled(enabled, button)
}

/// Filter-Chip (Pillenform); aktiv = Akzentfarbe.
pub fn chip(ui: &mut Ui, p: &Palette, text: &str, active: bool) -> Response {
    let (fill, color) = if active {
        (p.accent, Color32::WHITE)
    } else {
        (p.control_bg, p.text)
    };
    ui.add(
        Button::new(RichText::new(text).color(color))
            .fill(fill)
            .rounding(14.0),
    )
}

/// Schalter wie in den macOS-Systemeinstellungen.
pub fn toggle(ui: &mut Ui, p: &Palette, on: &mut bool) -> Response {
    let size = Vec2::new(38.0, 22.0);
    let (rect, mut response) = ui.allocate_exact_size(size, Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let how_on = ui.ctx().animate_bool(response.id, *on);
        let radius = rect.height() / 2.0;
        let background = if *on { p.success } else { p.control_hover };
        ui.painter().rect_filled(rect, radius, background);
        let x = egui::lerp((rect.left() + radius)..=(rect.right() - radius), how_on);
        ui.painter()
            .circle_filled(egui::pos2(x, rect.center().y), radius - 2.0, Color32::WHITE);
    }
    response
}

/// Zeile einer Auswahlliste (Seitenleiste, Notizliste): volle Breite, abgerundete Markierung.
pub fn nav_item(ui: &mut Ui, p: &Palette, label: &str, selected: bool) -> Response {
    let size = Vec2::new(ui.available_width(), 30.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    if ui.is_rect_visible(rect) {
        if selected {
            ui.painter().rect_filled(rect, CONTROL_RADIUS, p.selection);
        } else if response.hovered() {
            ui.painter()
                .rect_filled(rect, CONTROL_RADIUS, p.selection.gamma_multiply(0.5));
        }
        let galley =
            ui.painter()
                .layout_no_wrap(label.to_owned(), FontId::proportional(14.0), p.text);
        // Zu lange Beschriftungen am rechten Rand abschneiden.
        let text_rect = rect.shrink2(Vec2::new(10.0, 0.0));
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
