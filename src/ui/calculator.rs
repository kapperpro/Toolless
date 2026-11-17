use crate::i18n::t;
use eframe::egui::{self, Align, Button, Event, Frame, Key, Layout, Margin, RichText, Stroke, Ui};

use crate::{
    app::Shared,
    services::calc::{evaluate, format_number},
    theme::{self, Palette},
};

const GAP: f32 = 4.0;
/// Breite der Verlaufsspalte neben dem Tastenfeld.
const HISTORY_WIDTH: f32 = 250.0;
const OPERATORS: &str = "+−×÷";
/// Internes Kommando der Rücktaste (auf der Taste steht „←“).
const BACKSPACE: &str = "⌫";

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Function,
    Operator,
    Digit,
}

/// Tastenfeld: (Beschriftung, Kommando, Art).
const ROWS: [[(&str, &str, Kind); 4]; 5] = [
    [
        ("C", "AC", Kind::Function),
        ("(", "(", Kind::Function),
        (")", ")", Kind::Function),
        ("←", BACKSPACE, Kind::Function),
    ],
    [
        ("7", "7", Kind::Digit),
        ("8", "8", Kind::Digit),
        ("9", "9", Kind::Digit),
        ("÷", "÷", Kind::Operator),
    ],
    [
        ("4", "4", Kind::Digit),
        ("5", "5", Kind::Digit),
        ("6", "6", Kind::Digit),
        ("×", "×", Kind::Operator),
    ],
    [
        ("1", "1", Kind::Digit),
        ("2", "2", Kind::Digit),
        ("3", "3", Kind::Digit),
        ("−", "−", Kind::Operator),
    ],
    [
        ("±", "±", Kind::Digit),
        ("0", "0", Kind::Digit),
        (",", ",", Kind::Digit),
        ("+", "+", Kind::Operator),
    ],
];

#[derive(Default)]
pub struct CalculatorView {
    expression: String,
    error: Option<String>,
    /// Tastengröße und Verlaufsbreite – werden pro Frame aus dem verfügbaren Platz berechnet.
    key_w: f32,
    key_h: f32,
    history_w: f32,
    /// Nach „=“ ersetzt die nächste Zifferneingabe das Ergebnis.
    just_evaluated: bool,
    history: Vec<(String, String)>,
}

impl CalculatorView {
    fn press(&mut self, key: &str) {
        self.error = None;
        match key {
            "AC" => {
                self.expression.clear();
                self.just_evaluated = false;
            }
            "⌫" => {
                if self.just_evaluated {
                    self.expression.clear();
                } else {
                    self.expression.pop();
                }
                self.just_evaluated = false;
            }
            "=" => {
                if self.expression.is_empty() {
                    return;
                }
                match evaluate(&self.expression) {
                    Ok(value) => {
                        let result = format_number(value);
                        self.history
                            .insert(0, (self.expression.clone(), result.clone()));
                        self.history.truncate(30);
                        self.expression = result;
                        self.just_evaluated = true;
                    }
                    Err(message) => self.error = Some(message),
                }
            }
            "±" => {
                let e = &self.expression;
                let is_number =
                    |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit() || c == ',');
                self.expression = if e.is_empty() {
                    "−".to_owned()
                } else if let Some(inner) = e.strip_prefix("−(").and_then(|r| r.strip_suffix(')'))
                {
                    inner.to_owned()
                } else if let Some(rest) = e.strip_prefix('−').filter(|r| is_number(r)) {
                    rest.to_owned()
                } else if is_number(e) {
                    format!("−{e}")
                } else {
                    format!("−({e})")
                };
                self.just_evaluated = false;
            }
            _ => {
                let is_operator = OPERATORS.contains(key);
                if self.just_evaluated && !is_operator && key != "%" {
                    // Neues Rechnen nach „=“: Ziffern ersetzen das Ergebnis.
                    self.expression = key.to_owned();
                } else {
                    // Zwei Operatoren hintereinander: der letzte gewinnt (außer Minus als Vorzeichen).
                    let ends_with_operator = self
                        .expression
                        .chars()
                        .last()
                        .is_some_and(|c| OPERATORS.contains(c));
                    if is_operator && ends_with_operator && key != "−" {
                        self.expression.pop();
                    }
                    self.expression.push_str(key);
                }
                self.just_evaluated = false;
            }
        }
    }

    /// Tastatureingabe wie bei einem nativen Rechner.
    fn handle_keyboard(&mut self, ui: &Ui) {
        let events = ui.input(|i| i.events.clone());
        for event in events {
            match event {
                Event::Text(text) => {
                    for c in text.chars() {
                        let key = match c {
                            '0'..='9' | '+' | '(' | ')' | '%' | '=' => c.to_string(),
                            '*' => "×".to_owned(),
                            '/' => "÷".to_owned(),
                            '-' => "−".to_owned(),
                            '.' | ',' => ",".to_owned(),
                            _ => continue,
                        };
                        self.press(&key);
                    }
                }
                Event::Key {
                    key, pressed: true, ..
                } => match key {
                    Key::Enter => self.press("="),
                    Key::Backspace => self.press("⌫"),
                    Key::Escape | Key::Delete => self.press("AC"),
                    _ => {}
                },
                _ => {}
            }
        }
    }

    fn key_button(
        &mut self,
        ui: &mut Ui,
        p: &Palette,
        label: &str,
        command: &str,
        kind: Kind,
        width: f32,
    ) {
        let (fill, color) = match kind {
            Kind::Digit => (p.control_bg, p.text),
            Kind::Function | Kind::Operator => (p.field_bg, p.text),
        };
        let (fill, color) = if command == "=" {
            (p.accent, p.on_accent)
        } else {
            (fill, color)
        };
        let button = Button::new(
            RichText::new(label)
                .size((self.key_h * 0.32).clamp(16.0, 22.0))
                .color(color),
        )
        .fill(fill)
        .stroke(Stroke::new(1.0_f32, p.separator.gamma_multiply(0.6)))
        .rounding(p.radius);
        if ui.add_sized([width, self.key_h], button).clicked() {
            self.press(command);
        }
    }

    fn keypad(&mut self, ui: &mut Ui, p: &Palette) {
        let key_w = self.key_w;
        let pad_width = 4.0 * key_w + 3.0 * GAP;
        ui.set_width(pad_width);
        ui.spacing_mut().item_spacing = egui::vec2(GAP, GAP);

        // Anzeige: oben Vorschau/Fehler, darunter der Ausdruck.
        let preview = match (&self.error, self.just_evaluated) {
            (Some(error), _) => error.clone(),
            (None, false) if !self.expression.is_empty() => evaluate(&self.expression)
                .map(|v| format!("= {}", format_number(v)))
                .unwrap_or_default(),
            _ => String::new(),
        };
        Frame::none()
            .fill(p.field_bg)
            .rounding(p.radius)
            .inner_margin(Margin::symmetric(10.0, 8.0))
            .show(ui, |ui| {
                ui.set_width(pad_width - 20.0);
                ui.with_layout(Layout::top_down(Align::Max), |ui| {
                    let preview_color = if self.error.is_some() {
                        p.danger
                    } else {
                        p.text_secondary
                    };
                    ui.label(RichText::new(preview).size(13.0).color(preview_color));
                    let shown = if self.expression.is_empty() {
                        "0"
                    } else {
                        &self.expression
                    };
                    // Lange Ausdrücke: das Ende bleibt sichtbar.
                    let tail: String = {
                        let chars: Vec<char> = shown.chars().collect();
                        let start = chars.len().saturating_sub(15);
                        chars[start..].iter().collect()
                    };
                    ui.label(RichText::new(tail).size(30.0).color(p.text));
                });
            });
        ui.add_space(4.0);

        for row in ROWS {
            ui.horizontal(|ui| {
                for (label, command, kind) in row {
                    self.key_button(ui, p, label, command, kind, key_w);
                }
            });
        }
        ui.horizontal(|ui| {
            self.key_button(ui, p, "%", "%", Kind::Function, key_w);
            self.key_button(ui, p, "=", "=", Kind::Operator, 3.0 * key_w + 2.0 * GAP);
        });
    }

    fn history_panel(&mut self, ui: &mut Ui, p: &Palette) {
        ui.set_width(self.history_w);
        ui.horizontal(|ui| {
            theme::card_title(ui, t("Verlauf"));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add_enabled(!self.history.is_empty(), Button::new(t("Verlauf löschen")))
                    .clicked()
                {
                    self.history.clear();
                }
            });
        });
        if self.history.is_empty() {
            ui.label(theme::secondary(p, t("Noch keine Rechnungen.")));
            return;
        }

        let mut reuse = None;
        let mut remove = None;
        egui::ScrollArea::vertical()
            .max_height(300.0)
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for (index, (expression, result)) in self.history.iter().enumerate() {
                    let row = theme::clickable_row(ui, p, 44.0, false, |ui| {
                        ui.with_layout(Layout::top_down(Align::Max), |ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.add_space(3.0);
                            ui.label(theme::secondary(p, expression.clone()).size(12.5));
                            ui.label(RichText::new(format!("= {result}")).strong());
                        });
                    })
                    .on_hover_text(t(
                        "Klick übernimmt das Ergebnis · Rechtsklick entfernt den Eintrag",
                    ));
                    if row.clicked() {
                        reuse = Some(result.clone());
                    }
                    row.context_menu(|ui| {
                        if ui.button(t("Ergebnis übernehmen")).clicked() {
                            reuse = Some(result.clone());
                            ui.close_menu();
                        }
                        if ui.button(t("Eintrag entfernen")).clicked() {
                            remove = Some(index);
                            ui.close_menu();
                        }
                    });
                }
            });
        if let Some(result) = reuse {
            self.expression = result;
            self.just_evaluated = true;
            self.error = None;
        }
        if let Some(index) = remove {
            self.history.remove(index);
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, shared: &mut Shared) {
        self.handle_keyboard(ui);
        let p = shared.palette;

        // Tastenfeld und Verlauf teilen sich den Platz: breit nebeneinander, schmal untereinander.
        // Die Tasten wachsen mit dem Fenster, bleiben aber in einem angenehmen Bereich.
        const CARD_PADDING: f32 = 30.0;
        let available = ui.available_size();
        let side_by_side = available.x >= 620.0;
        let pad_width = if side_by_side {
            (available.x - HISTORY_WIDTH - 2.0 * CARD_PADDING - 8.0).clamp(250.0, 780.0)
        } else {
            (available.x - CARD_PADDING).clamp(220.0, 520.0)
        };
        self.key_w = (pad_width - 3.0 * GAP) / 4.0;
        let height_for_keys = if available.y.is_finite() {
            available.y - 150.0
        } else {
            320.0
        };
        self.key_h = (height_for_keys / 6.0 - GAP).clamp(34.0, 72.0);
        self.history_w = if side_by_side {
            HISTORY_WIDTH
        } else {
            pad_width
        };

        if side_by_side {
            ui.horizontal_top(|ui| {
                // Karten liegen in einer waagerechten Reihe – ihr Inhalt läuft senkrecht.
                theme::card(&p).show(ui, |ui| {
                    ui.vertical(|ui| self.keypad(ui, &p));
                });
                theme::card(&p).show(ui, |ui| {
                    ui.vertical(|ui| self.history_panel(ui, &p));
                });
            });
        } else {
            theme::card(&p).show(ui, |ui| self.keypad(ui, &p));
            theme::card(&p).show(ui, |ui| self.history_panel(ui, &p));
        }
    }
}
