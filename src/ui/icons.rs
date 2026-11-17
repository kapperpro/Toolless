//! Kleine Symbole der Werkzeuge, mit Linien gezeichnet – unabhängig von installierten
//! Schriftarten und in jeder Akzentfarbe scharf.

use eframe::egui::{self, Color32, Painter, Pos2, Rect, Stroke, Vec2};

use crate::app::Tab;

/// Zeichnet das Symbol von `tab` mittig bei `center` (Kantenlänge etwa `size`).
pub fn draw(painter: &Painter, center: Pos2, size: f32, tab: Tab, color: Color32) {
    let u = size / 20.0; // Einheit: 1/20 der Kantenlänge
    let stroke = Stroke::new((1.5 * u).max(1.2), color);
    let at = |x: f32, y: f32| center + Vec2::new(x * u, y * u);
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(at(x0, y0), at(x1, y1));
    let line = |a: Pos2, b: Pos2| painter.line_segment([a, b], stroke);
    let round = |r: f32| (r * u).max(0.0);

    match tab {
        Tab::Spotify => {
            // Notenzeichen: Kopf, Hals, Fähnchen.
            painter.circle_filled(at(-3.5, 6.0), round(3.2), color);
            line(at(-0.5, 6.0), at(-0.5, -7.0));
            line(at(-0.5, -7.0), at(6.0, -4.0));
            line(at(6.0, -4.0), at(6.0, -1.0));
        }
        Tab::Clipboard => {
            painter.rect_stroke(rect(-6.0, -7.0, 6.0, 8.0), round(2.0), stroke);
            painter.rect_filled(rect(-3.0, -9.0, 3.0, -5.0), round(1.5), color);
            line(at(-3.0, 0.0), at(3.0, 0.0));
            line(at(-3.0, 3.5), at(3.0, 3.5));
        }
        Tab::Notes => {
            painter.rect_stroke(rect(-7.0, -8.0, 7.0, 8.0), round(2.0), stroke);
            line(at(-3.5, -3.5), at(3.5, -3.5));
            line(at(-3.5, 0.0), at(3.5, 0.0));
            line(at(-3.5, 3.5), at(1.0, 3.5));
        }
        Tab::Calculator => {
            painter.rect_stroke(rect(-6.5, -8.0, 6.5, 8.0), round(2.0), stroke);
            painter.rect_stroke(rect(-4.0, -5.5, 4.0, -2.5), round(0.8), stroke);
            for (x, y) in [
                (-3.0, 1.0),
                (0.0, 1.0),
                (3.0, 1.0),
                (-3.0, 4.5),
                (0.0, 4.5),
                (3.0, 4.5),
            ] {
                painter.circle_filled(at(x, y), round(0.9), color);
            }
        }
        Tab::Launcher => {
            for (x, y) in [(-7.0, -7.0), (1.0, -7.0), (-7.0, 1.0), (1.0, 1.0)] {
                painter.rect_stroke(rect(x, y, x + 6.0, y + 6.0), round(1.5), stroke);
            }
        }
        Tab::Monitor => {
            // Kurve mit Rahmen, wie die Leistungsansicht des Task-Managers.
            painter.rect_stroke(rect(-8.0, -7.0, 8.0, 7.0), round(2.0), stroke);
            painter.add(egui::Shape::line(
                vec![
                    at(-6.0, 3.0),
                    at(-3.0, 3.0),
                    at(-1.0, -3.5),
                    at(1.5, 4.0),
                    at(3.0, 0.0),
                    at(6.0, 0.0),
                ],
                stroke,
            ));
        }
        Tab::Minecraft => {
            // Würfel in Schrägansicht.
            let top = [at(0.0, -8.0), at(7.5, -4.0), at(0.0, 0.0), at(-7.5, -4.0)];
            painter.add(egui::Shape::closed_line(top.to_vec(), stroke));
            line(at(-7.5, -4.0), at(-7.5, 4.5));
            line(at(7.5, -4.0), at(7.5, 4.5));
            line(at(0.0, 0.0), at(0.0, 8.5));
            line(at(-7.5, 4.5), at(0.0, 8.5));
            line(at(7.5, 4.5), at(0.0, 8.5));
        }
        Tab::Games => {
            // Spielcontroller.
            painter.rect_stroke(rect(-9.0, -5.0, 9.0, 6.0), round(5.0), stroke);
            line(at(-5.5, -1.0), at(-1.5, -1.0));
            line(at(-3.5, -3.0), at(-3.5, 1.0));
            painter.circle_filled(at(3.0, 1.0), round(1.1), color);
            painter.circle_filled(at(5.5, -1.5), round(1.1), color);
        }
        Tab::Clock => {
            painter.circle_stroke(at(0.0, 0.0), round(8.5), stroke);
            line(at(0.0, 0.0), at(0.0, -5.0));
            line(at(0.0, 0.0), at(3.5, 2.0));
        }
        Tab::System => {
            // Zahnrad: Ring mit acht Zähnen.
            painter.circle_stroke(at(0.0, 0.0), round(4.0), stroke);
            for step in 0..8 {
                let angle = step as f32 * std::f32::consts::FRAC_PI_4;
                let (sin, cos) = angle.sin_cos();
                line(at(cos * 6.0, sin * 6.0), at(cos * 8.5, sin * 8.5));
            }
        }
    }
}

/// Drei waagerechte Striche („Hamburger“) zum Ein- und Ausklappen.
pub fn menu(painter: &Painter, center: Pos2, size: f32, color: Color32) {
    let u = size / 20.0;
    let stroke = Stroke::new((1.6 * u).max(1.2), color);
    for dy in [-5.0, 0.0, 5.0] {
        painter.line_segment(
            [
                center + Vec2::new(-7.0 * u, dy * u),
                center + Vec2::new(7.0 * u, dy * u),
            ],
            stroke,
        );
    }
}
