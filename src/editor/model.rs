//! What an edit consists of: a list of annotations over the untouched image.
//!
//! Nothing is burnt into the pixels until the result is saved or copied, so
//! every step can be undone, moved or restyled. All coordinates are image
//! pixels.

use egui::{Color32, Pos2, Rect, Vec2, pos2, vec2};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    Select,
    Rectangle,
    Ellipse,
    Arrow,
    Line,
    Pen,
    Highlighter,
    Text,
    Counter,
    Spotlight,
    Blur,
    Pixelate,
    Redact,
    Crop,
}

impl Tool {
    pub const ALL: [Tool; 14] = [
        Tool::Select,
        Tool::Rectangle,
        Tool::Ellipse,
        Tool::Arrow,
        Tool::Line,
        Tool::Pen,
        Tool::Highlighter,
        Tool::Text,
        Tool::Counter,
        Tool::Spotlight,
        Tool::Blur,
        Tool::Pixelate,
        Tool::Redact,
        Tool::Crop,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Rectangle => "Rectangle",
            Tool::Ellipse => "Ellipse",
            Tool::Arrow => "Arrow",
            Tool::Line => "Line",
            Tool::Pen => "Pen",
            Tool::Highlighter => "Highlighter",
            Tool::Text => "Text",
            Tool::Counter => "Step number",
            Tool::Spotlight => "Spotlight",
            Tool::Blur => "Blur",
            Tool::Pixelate => "Pixelate",
            Tool::Redact => "Black out",
            Tool::Crop => "Crop",
        }
    }

    pub fn key(self) -> egui::Key {
        use egui::Key;
        match self {
            Tool::Select => Key::V,
            Tool::Rectangle => Key::R,
            Tool::Ellipse => Key::E,
            Tool::Arrow => Key::A,
            Tool::Line => Key::L,
            Tool::Pen => Key::P,
            Tool::Highlighter => Key::H,
            Tool::Text => Key::T,
            Tool::Counter => Key::N,
            Tool::Spotlight => Key::S,
            Tool::Blur => Key::B,
            Tool::Pixelate => Key::X,
            Tool::Redact => Key::D,
            Tool::Crop => Key::C,
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Tool::Select => "Click to select, drag to move, drag a corner to resize. Del removes.",
            Tool::Rectangle | Tool::Ellipse => "Drag to draw. Shift keeps it square.",
            Tool::Arrow | Tool::Line => "Drag to draw. Shift snaps to 45°.",
            Tool::Pen | Tool::Highlighter => "Drag to draw freehand.",
            Tool::Text => "Click to place text, type, then click elsewhere or press Esc.",
            Tool::Counter => "Click to place the next number.",
            Tool::Spotlight => "Drag over what matters; everything else is dimmed.",
            Tool::Blur | Tool::Pixelate => "Drag over what should be unreadable.",
            Tool::Redact => "Drag to cover with a solid black box.",
            Tool::Crop => "Drag the part to keep, then press Enter.",
        }
    }

    /// Which settings mean something for this tool.
    pub fn uses(self) -> Uses {
        let none = Uses::default();
        match self {
            Tool::Rectangle | Tool::Ellipse => Uses {
                color: true,
                width: true,
                fill: true,
                ..none
            },
            Tool::Arrow | Tool::Line | Tool::Pen | Tool::Highlighter => Uses {
                color: true,
                width: true,
                ..none
            },
            Tool::Text => Uses {
                color: true,
                fill: true,
                font: true,
                ..none
            },
            Tool::Counter => Uses {
                color: true,
                font: true,
                ..none
            },
            Tool::Spotlight => Uses {
                strength: true,
                round: true,
                ..none
            },
            Tool::Blur | Tool::Pixelate => Uses {
                strength: true,
                ..none
            },
            Tool::Select | Tool::Redact | Tool::Crop => none,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Uses {
    pub color: bool,
    pub width: bool,
    pub fill: bool,
    pub font: bool,
    pub strength: bool,
    pub round: bool,
}

/// How an annotation looks. Every annotation keeps its own copy, so changing
/// the settings later only affects what is drawn next or what is selected.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Style {
    pub color: Color32,
    /// `None` for an outline only.
    pub fill: Option<Color32>,
    /// Line width in image pixels.
    pub width: f32,
    /// Text height in image pixels.
    pub font_size: f32,
    /// Blur radius, pixel block size or spotlight dimming, depending on the
    /// tool; 0 to 1 for the spotlight.
    pub strength: f32,
    /// Spotlight as an ellipse rather than a rectangle.
    pub round: bool,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            color: Color32::from_rgb(0xe5, 0x39, 0x35),
            fill: None,
            width: 4.0,
            font_size: 28.0,
            strength: 12.0,
            round: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Rectangle(Rect),
    Ellipse(Rect),
    Arrow(Pos2, Pos2),
    Line(Pos2, Pos2),
    Pen(Vec<Pos2>),
    Highlighter(Vec<Pos2>),
    /// Top left corner and the text, which may span several lines.
    Text(Pos2, String),
    /// Centre and number.
    Counter(Pos2, u32),
    Spotlight(Rect),
    Blur(Rect),
    Pixelate(Rect),
    Redact(Rect),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Annotation {
    pub shape: Shape,
    pub style: Style,
}

/// How a highlighter stroke relates to the chosen width and colour.
pub const HIGHLIGHTER_WIDTH: f32 = 4.0;
pub const HIGHLIGHTER_ALPHA: u8 = 110;

/// Size of the circle behind a step number, relative to the font size.
pub const COUNTER_RADIUS: f32 = 0.8;

impl Annotation {
    /// Everything that can be dragged by its corners.
    pub fn rect(&self) -> Option<Rect> {
        match &self.shape {
            Shape::Rectangle(r)
            | Shape::Ellipse(r)
            | Shape::Spotlight(r)
            | Shape::Blur(r)
            | Shape::Pixelate(r)
            | Shape::Redact(r) => Some(*r),
            _ => None,
        }
    }

    pub fn set_rect(&mut self, rect: Rect) {
        match &mut self.shape {
            Shape::Rectangle(r)
            | Shape::Ellipse(r)
            | Shape::Spotlight(r)
            | Shape::Blur(r)
            | Shape::Pixelate(r)
            | Shape::Redact(r) => *r = rect,
            _ => {}
        }
    }

    pub fn endpoints(&self) -> Option<(Pos2, Pos2)> {
        match self.shape {
            Shape::Arrow(a, b) | Shape::Line(a, b) => Some((a, b)),
            _ => None,
        }
    }

    pub fn set_endpoints(&mut self, from: Pos2, to: Pos2) {
        if let Shape::Arrow(a, b) | Shape::Line(a, b) = &mut self.shape {
            *a = from;
            *b = to;
        }
    }

    /// The area it covers, strokes included. Text needs its measured size,
    /// which only the renderer knows.
    pub fn bounds(&self, text_size: Option<Vec2>) -> Rect {
        let half = self.stroke_width() / 2.0;
        match &self.shape {
            Shape::Rectangle(r) | Shape::Ellipse(r) => r.expand(half),
            Shape::Spotlight(r) | Shape::Blur(r) | Shape::Pixelate(r) | Shape::Redact(r) => *r,
            Shape::Arrow(a, b) | Shape::Line(a, b) => {
                Rect::from_two_pos(*a, *b).expand(half.max(self.head_length() / 2.0))
            }
            Shape::Pen(points) | Shape::Highlighter(points) => {
                Rect::from_points(points).expand(half)
            }
            Shape::Text(at, _) => {
                Rect::from_min_size(*at, text_size.unwrap_or(vec2(20.0, self.style.font_size)))
            }
            Shape::Counter(c, _) => {
                Rect::from_center_size(*c, Vec2::splat(self.counter_radius() * 2.0))
            }
        }
    }

    pub fn stroke_width(&self) -> f32 {
        match self.shape {
            Shape::Highlighter(_) => self.style.width * HIGHLIGHTER_WIDTH,
            _ => self.style.width,
        }
    }

    pub fn head_length(&self) -> f32 {
        arrow_head_length(self.style.width)
    }

    pub fn counter_radius(&self) -> f32 {
        self.style.font_size * COUNTER_RADIUS
    }

    /// Whether a click at `p` means this annotation. `tolerance` is in image
    /// pixels and should correspond to a few screen pixels.
    pub fn hit(&self, p: Pos2, tolerance: f32, text_size: Option<Vec2>) -> bool {
        let reach = tolerance + self.stroke_width() / 2.0;
        match &self.shape {
            Shape::Rectangle(r) => {
                if self.style.fill.is_some() && r.contains(p) {
                    return true;
                }
                let outer = r.expand(reach);
                let inner = r.shrink(reach);
                outer.contains(p) && !(inner.is_positive() && inner.contains(p))
            }
            Shape::Ellipse(r) => {
                let c = r.center();
                let radius = r.size() / 2.0;
                if radius.x <= 0.0 || radius.y <= 0.0 {
                    return false;
                }
                let d = vec2((p.x - c.x) / radius.x, (p.y - c.y) / radius.y).length();
                if self.style.fill.is_some() && d <= 1.0 {
                    return true;
                }
                (d - 1.0).abs() * radius.x.min(radius.y) <= reach
            }
            Shape::Arrow(a, b) | Shape::Line(a, b) => segment_distance(p, *a, *b) <= reach,
            Shape::Pen(points) | Shape::Highlighter(points) => {
                if points.len() == 1 {
                    return (points[0] - p).length() <= reach;
                }
                points
                    .windows(2)
                    .any(|w| segment_distance(p, w[0], w[1]) <= reach)
            }
            Shape::Text(..) | Shape::Counter(..) => {
                self.bounds(text_size).expand(tolerance).contains(p)
            }
            Shape::Spotlight(r) | Shape::Blur(r) | Shape::Pixelate(r) | Shape::Redact(r) => {
                r.expand(tolerance).contains(p)
            }
        }
    }

    pub fn translate(&mut self, d: Vec2) {
        match &mut self.shape {
            Shape::Rectangle(r)
            | Shape::Ellipse(r)
            | Shape::Spotlight(r)
            | Shape::Blur(r)
            | Shape::Pixelate(r)
            | Shape::Redact(r) => *r = r.translate(d),
            Shape::Arrow(a, b) | Shape::Line(a, b) => {
                *a += d;
                *b += d;
            }
            Shape::Pen(points) | Shape::Highlighter(points) => {
                for p in points {
                    *p += d;
                }
            }
            Shape::Text(at, _) | Shape::Counter(at, _) => *at += d,
        }
    }
}

pub fn arrow_head_length(width: f32) -> f32 {
    (width * 3.0 + 10.0).max(12.0)
}

pub fn segment_distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_sq();
    if len2 <= f32::EPSILON {
        return (p - a).length();
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    (p - (a + ab * t)).length()
}

/// A rectangle from two corners, as a square when `square` is set.
pub fn drag_rect(from: Pos2, to: Pos2, square: bool) -> Rect {
    if !square {
        return Rect::from_two_pos(from, to);
    }
    let d = to - from;
    let side = d.x.abs().max(d.y.abs());
    let to = pos2(from.x + side * d.x.signum(), from.y + side * d.y.signum());
    Rect::from_two_pos(from, to)
}

/// The end of a line, snapped to the nearest 45° when `snap` is set.
pub fn snap_line(from: Pos2, to: Pos2, snap: bool) -> Pos2 {
    if !snap {
        return to;
    }
    let d = to - from;
    let step = std::f32::consts::FRAC_PI_4;
    let angle = (d.y.atan2(d.x) / step).round() * step;
    from + vec2(angle.cos(), angle.sin()) * d.length()
}

/// The whole edit: annotations in drawing order and the crop, if any.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Doc {
    pub items: Vec<Annotation>,
    pub crop: Option<Rect>,
}

impl Doc {
    /// The number the next step marker gets.
    pub fn next_counter(&self) -> u32 {
        self.items
            .iter()
            .filter_map(|a| match a.shape {
                Shape::Counter(_, n) => Some(n),
                _ => None,
            })
            .max()
            .unwrap_or(0)
            + 1
    }
}

/// Undo and redo as whole snapshots. An edit is a few dozen annotations at
/// most, so copying the list is cheaper than anything cleverer.
#[derive(Default)]
pub struct History {
    undo: Vec<Doc>,
    redo: Vec<Doc>,
}

const HISTORY_LIMIT: usize = 200;

impl History {
    /// Call before changing `doc`, with the state it has now.
    pub fn record(&mut self, before: &Doc) {
        self.undo.push(before.clone());
        if self.undo.len() > HISTORY_LIMIT {
            self.undo.remove(0);
        }
        self.redo.clear();
    }

    pub fn undo(&mut self, doc: &mut Doc) -> bool {
        match self.undo.pop() {
            Some(previous) => {
                self.redo.push(std::mem::replace(doc, previous));
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self, doc: &mut Doc) -> bool {
        match self.redo.pop() {
            Some(next) => {
                self.undo.push(std::mem::replace(doc, next));
                true
            }
            None => false,
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(shape: Shape) -> Annotation {
        Annotation {
            shape,
            style: Style::default(),
        }
    }

    #[test]
    fn outlines_are_hit_on_the_line_not_inside() {
        let r = item(Shape::Rectangle(Rect::from_min_size(
            pos2(10.0, 10.0),
            vec2(100.0, 50.0),
        )));
        assert!(r.hit(pos2(10.0, 30.0), 2.0, None));
        assert!(!r.hit(pos2(60.0, 35.0), 2.0, None));

        let mut filled = r.clone();
        filled.style.fill = Some(Color32::WHITE);
        assert!(filled.hit(pos2(60.0, 35.0), 2.0, None));

        let e = item(Shape::Ellipse(Rect::from_min_size(
            pos2(0.0, 0.0),
            vec2(100.0, 100.0),
        )));
        assert!(e.hit(pos2(50.0, 0.0), 2.0, None));
        assert!(!e.hit(pos2(50.0, 50.0), 2.0, None));
    }

    #[test]
    fn lines_and_freehand_are_hit_near_the_stroke() {
        let l = item(Shape::Arrow(pos2(0.0, 0.0), pos2(100.0, 0.0)));
        assert!(l.hit(pos2(50.0, 3.0), 2.0, None));
        assert!(!l.hit(pos2(50.0, 20.0), 2.0, None));

        let pen = item(Shape::Pen(vec![
            pos2(0.0, 0.0),
            pos2(10.0, 10.0),
            pos2(20.0, 0.0),
        ]));
        assert!(pen.hit(pos2(10.0, 10.0), 1.0, None));
        assert!(!pen.hit(pos2(10.0, 0.0), 1.0, None));
    }

    #[test]
    fn shift_constrains_shapes() {
        let r = drag_rect(pos2(0.0, 0.0), pos2(30.0, -10.0), true);
        assert_eq!(r.width(), r.height());
        let end = snap_line(pos2(0.0, 0.0), pos2(100.0, 8.0), true);
        assert!(end.y.abs() < 1e-3);
    }

    #[test]
    fn undo_and_redo_walk_the_snapshots() {
        let mut doc = Doc::default();
        let mut history = History::default();
        history.record(&doc);
        doc.items.push(item(Shape::Counter(pos2(1.0, 1.0), 1)));
        assert_eq!(doc.next_counter(), 2);
        assert!(history.undo(&mut doc));
        assert!(doc.items.is_empty());
        assert!(history.redo(&mut doc));
        assert_eq!(doc.items.len(), 1);
        assert!(!history.redo(&mut doc));
    }
}
