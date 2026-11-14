use eframe::egui::{self, Align, Button, Color32, Event, Frame, Key, Layout, Margin, RichText, Ui};

use crate::{
    app::Shared,
    services::calc::{evaluate, format_number},
    theme,
};

const KEY_W: f32 = 58.0;
const KEY_H: f32 = 50.0;
const GAP: f32 = 8.0;
const OPERATORS: &str = "+−×÷";

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Function,
    Operator,
    Digit,
}

const ROWS: [[(&str, Kind); 4]; 5] = [
    [
        ("AC", Kind::Function),
        ("(", Kind::Function),
        (")", Kind::Function),
        ("÷", Kind::Operator),
    ],
    [
        ("7", Kind::Digit),
        ("8", Kind::Digit),
        ("9", Kind::Digit),
        ("×", Kind::Operator),
    ],
    [
        ("4", Kind::Digit),
        ("5", Kind::Digit),
        ("6", Kind::Digit),
        ("−", Kind::Operator),
    ],
    [
        ("1", Kind::Digit),
        ("2", Kind::Digit),
        ("3", Kind::Digit),
        ("+", Kind::Operator),
    ],
    [
        ("±", Kind::Digit),
        ("0", Kind::Digit),
        (",", Kind::Digit),
        ("=", Kind::Operator),
    ],
];

#[derive(Default)]
pub struct CalculatorView {
    expression: String,
    error: Option<String>,
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

    fn key_button(&mut self, ui: &mut Ui, label: &str, kind: Kind, width: f32) {
        let fill = match kind {
            Kind::Function => Color32::from_gray(99),
            Kind::Operator => Color32::from_rgb(255, 159, 10),
            Kind::Digit => Color32::from_gray(58),
        };
        let button = Button::new(RichText::new(label).size(20.0).color(Color32::WHITE))
            .fill(fill)
            .rounding(KEY_H / 2.0);
        if ui.add_sized([width, KEY_H], button).clicked() {
            self.press(label);
        }
    }

    pub fn ui(&mut self, ui: &mut Ui, shared: &mut Shared) {
        self.handle_keyboard(ui);
        let p = shared.palette;
        let pad_width = 4.0 * KEY_W + 3.0 * GAP;

        ui.horizontal_top(|ui| {
            Frame::none()
                .fill(Color32::from_rgb(28, 28, 30))
                .rounding(16.0)
                .inner_margin(Margin::same(14.0))
                .show(ui, |ui| {
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
                    ui.with_layout(Layout::top_down(Align::Max), |ui| {
                        ui.label(
                            RichText::new(preview)
                                .size(13.0)
                                .color(Color32::from_gray(150)),
                        );
                        let shown = if self.expression.is_empty() {
                            "0"
                        } else {
                            &self.expression
                        };
                        // Lange Ausdrücke: das Ende bleibt sichtbar.
                        let tail: String = {
                            let chars: Vec<char> = shown.chars().collect();
                            let start = chars.len().saturating_sub(13);
                            chars[start..].iter().collect()
                        };
                        ui.label(RichText::new(tail).size(32.0).color(Color32::WHITE));
                    });
                    ui.add_space(4.0);

                    for row in ROWS {
                        ui.horizontal(|ui| {
                            for (label, kind) in row {
                                self.key_button(ui, label, kind, KEY_W);
                            }
                        });
                    }
                    ui.horizontal(|ui| {
                        let wide = 2.0 * KEY_W + GAP;
                        self.key_button(ui, "%", Kind::Function, wide);
                        // Anzeige „Löschen“, intern das Rücktasten-Kommando.
                        let button =
                            Button::new(RichText::new("Löschen").size(16.0).color(Color32::WHITE))
                                .fill(Color32::from_gray(99))
                                .rounding(KEY_H / 2.0);
                        if ui.add_sized([wide, KEY_H], button).clicked() {
                            self.press("⌫");
                        }
                    });
                });

            theme::card(&p).show(ui, |ui| {
                ui.set_width(230.0);
                theme::card_title(ui, "Verlauf");
                if self.history.is_empty() {
                    ui.label(theme::secondary(&p, "Noch keine Rechnungen."));
                }
                let mut reuse = None;
                egui::ScrollArea::vertical()
                    .max_height(360.0)
                    .show(ui, |ui| {
                        for (expression, result) in &self.history {
                            ui.label(theme::secondary(&p, expression.clone()));
                            let entry = Button::new(RichText::new(format!("= {result}")).strong())
                                .frame(false);
                            if ui.add(entry).on_hover_text("Ergebnis übernehmen").clicked() {
                                reuse = Some(result.clone());
                            }
                            ui.separator();
                        }
                    });
                if let Some(result) = reuse {
                    self.expression = result;
                    self.just_evaluated = true;
                    self.error = None;
                }
            });
        });
    }
}
