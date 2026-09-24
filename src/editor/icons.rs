//! Tool icons, drawn with shapes rather than loaded from image files. They
//! stay sharp at any display scaling and take the colour of the theme.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, vec2};

use super::model::Tool;

/// Draw the icon for `tool` into `rect`, which should be about 16 points.
pub fn paint(painter: &Painter, rect: Rect, tool: Tool, color: Color32) {
    let s = rect.width() / 16.0;
    // Coordinates below are on a 16 × 16 grid.
    let p = |x: f32, y: f32| -> Pos2 { rect.min + vec2(x, y) * s };
    let line = Stroke::new(1.4 * s, color);
    let thin = Stroke::new(1.0 * s, color);

    match tool {
        Tool::Select => {
            painter.add(Shape::convex_polygon(
                vec![p(3.0, 1.5), p(12.5, 10.0), p(8.0, 10.5), p(5.5, 14.5)],
                Color32::TRANSPARENT,
                line,
            ));
        }
        Tool::Rectangle => {
            painter.rect_stroke(
                Rect::from_min_max(p(2.0, 3.5), p(14.0, 12.5)),
                1.0 * s,
                line,
                egui::StrokeKind::Middle,
            );
        }
        Tool::Ellipse => {
            painter.add(Shape::ellipse_stroke(p(8.0, 8.0), vec2(6.2, 4.8) * s, line));
        }
        Tool::Arrow => {
            painter.line_segment([p(2.5, 13.5), p(11.0, 5.0)], line);
            painter.add(Shape::convex_polygon(
                vec![p(13.8, 2.2), p(12.8, 8.2), p(7.8, 3.2)],
                color,
                Stroke::NONE,
            ));
        }
        Tool::Line => {
            painter.line_segment([p(2.5, 13.5), p(13.5, 2.5)], line);
        }
        Tool::Pen => {
            let points: Vec<Pos2> = (0..=16)
                .map(|i| {
                    let t = i as f32 / 16.0;
                    p(
                        2.0 + t * 12.0,
                        9.0 + (t * std::f32::consts::TAU * 1.2).sin() * 3.5,
                    )
                })
                .collect();
            painter.add(Shape::line(points, line));
        }
        Tool::Highlighter => {
            painter.line_segment(
                [p(2.5, 11.0), p(13.5, 11.0)],
                Stroke::new(4.5 * s, color.gamma_multiply(0.45)),
            );
            painter.line_segment([p(5.0, 10.0), p(11.0, 3.0)], line);
            painter.line_segment([p(9.0, 11.5), p(13.0, 5.5)], thin);
        }
        Tool::Text => {
            painter.line_segment([p(3.0, 3.0), p(13.0, 3.0)], line);
            painter.line_segment([p(8.0, 3.0), p(8.0, 13.5)], line);
            painter.line_segment([p(6.0, 13.5), p(10.0, 13.5)], thin);
        }
        Tool::Counter => {
            painter.circle_stroke(p(8.0, 8.0), 6.0 * s, line);
            painter.line_segment([p(6.8, 6.2), p(8.3, 4.8)], line);
            painter.line_segment([p(8.3, 4.8), p(8.3, 11.3)], line);
        }
        Tool::Spotlight => {
            painter.rect_filled(
                Rect::from_min_max(p(1.0, 1.0), p(15.0, 15.0)),
                2.0 * s,
                color.gamma_multiply(0.25),
            );
            painter.circle_filled(p(8.0, 8.0), 4.2 * s, color);
        }
        Tool::Blur => {
            for (x, y, a) in [
                (4.5, 4.5, 0.35),
                (8.0, 4.5, 0.6),
                (11.5, 4.5, 0.35),
                (4.5, 8.0, 0.6),
                (8.0, 8.0, 1.0),
                (11.5, 8.0, 0.6),
                (4.5, 11.5, 0.35),
                (8.0, 11.5, 0.6),
                (11.5, 11.5, 0.35),
            ] {
                painter.circle_filled(p(x, y), 1.6 * s, color.gamma_multiply(a));
            }
        }
        Tool::Pixelate => {
            for (x, y, a) in [
                (0.0, 0.0, 1.0),
                (1.0, 0.0, 0.4),
                (2.0, 0.0, 0.7),
                (0.0, 1.0, 0.4),
                (1.0, 1.0, 0.8),
                (2.0, 1.0, 0.3),
                (0.0, 2.0, 0.7),
                (1.0, 2.0, 0.3),
                (2.0, 2.0, 1.0),
            ] {
                let min = p(2.0 + x * 4.0, 2.0 + y * 4.0);
                painter.rect_filled(
                    Rect::from_min_max(min, min + vec2(3.6, 3.6) * s),
                    0.0,
                    color.gamma_multiply(a),
                );
            }
        }
        Tool::Redact => {
            painter.rect_filled(
                Rect::from_min_max(p(1.5, 5.0), p(14.5, 11.0)),
                1.0 * s,
                color,
            );
        }
        Tool::Crop => {
            painter.add(Shape::line(
                vec![p(4.5, 1.5), p(4.5, 11.5), p(14.5, 11.5)],
                line,
            ));
            painter.add(Shape::line(
                vec![p(1.5, 4.5), p(11.5, 4.5), p(11.5, 14.5)],
                line,
            ));
        }
    }
}
