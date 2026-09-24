//! Zoom and pan over an image, shared by the viewer and the editor.
//!
//! The view remembers which image pixel sits in the middle of the area and
//! how many screen points one image pixel takes. "Fit" is a mode rather than a
//! number, so resizing the window keeps the whole image in view.

use egui::{Pos2, Rect, Vec2, pos2, vec2};

pub const MIN_SCALE: f32 = 0.02;
pub const MAX_SCALE: f32 = 40.0;
/// One notch of the mouse wheel.
pub const WHEEL_STEP: f32 = 1.2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// Points per image pixel, or `None` to fit the image into the area.
    scale: Option<f32>,
    /// The image point shown in the middle of the area.
    center: Pos2,
}

impl Default for View {
    fn default() -> Self {
        Self {
            scale: None,
            center: Pos2::ZERO,
        }
    }
}

impl View {
    pub fn fit(&mut self) {
        self.scale = None;
    }

    pub fn is_fit(&self) -> bool {
        self.scale.is_none()
    }

    /// Points per image pixel for an image of `size` shown in `area`. Fitting
    /// never enlarges: a small image is shown at its real size.
    pub fn scale(&self, area: Rect, size: Vec2, pixels_per_point: f32) -> f32 {
        match self.scale {
            Some(s) => s,
            None => fit_scale(area, size, pixels_per_point),
        }
    }

    fn center(&self, size: Vec2) -> Pos2 {
        if self.scale.is_none() {
            (size / 2.0).to_pos2()
        } else {
            self.center
        }
    }

    /// Where an image point lands on screen.
    pub fn screen_pos(&self, area: Rect, size: Vec2, ppp: f32, p: Pos2) -> Pos2 {
        let s = self.scale(area, size, ppp);
        area.center() + (p - self.center(size)) * s
    }

    /// Which image point is under a screen position.
    pub fn image_pos(&self, area: Rect, size: Vec2, ppp: f32, p: Pos2) -> Pos2 {
        let s = self.scale(area, size, ppp);
        self.center(size) + (p - area.center()) / s
    }

    /// The whole image in screen coordinates.
    pub fn image_rect(&self, area: Rect, size: Vec2, ppp: f32) -> Rect {
        Rect::from_min_max(
            self.screen_pos(area, size, ppp, Pos2::ZERO),
            self.screen_pos(area, size, ppp, size.to_pos2()),
        )
    }

    /// Set an absolute scale, keeping the image point under `anchor` where it is.
    pub fn set_scale(&mut self, area: Rect, size: Vec2, ppp: f32, scale: f32, anchor: Pos2) {
        let before = self.image_pos(area, size, ppp, anchor);
        let scale = scale.clamp(MIN_SCALE, MAX_SCALE);
        self.scale = Some(scale);
        self.center = before - (anchor - area.center()) / scale;
    }

    pub fn zoom_by(&mut self, area: Rect, size: Vec2, ppp: f32, factor: f32, anchor: Pos2) {
        let current = self.scale(area, size, ppp);
        self.set_scale(area, size, ppp, current * factor, anchor);
    }

    /// One image pixel on one screen pixel, centred on `anchor`.
    pub fn actual_size(&mut self, area: Rect, size: Vec2, ppp: f32, anchor: Pos2) {
        self.set_scale(area, size, ppp, 1.0 / ppp.max(0.1), anchor);
    }

    /// Move the image by a screen distance.
    pub fn pan(&mut self, area: Rect, size: Vec2, ppp: f32, delta: Vec2) {
        let s = self.scale(area, size, ppp);
        self.center = self.center(size) - delta / s;
        self.scale = Some(s);
    }

    /// The zoom as the user thinks of it: 100 % is one image pixel per screen
    /// pixel.
    pub fn percent(&self, area: Rect, size: Vec2, ppp: f32) -> f32 {
        self.scale(area, size, ppp) * ppp * 100.0
    }
}

fn fit_scale(area: Rect, size: Vec2, ppp: f32) -> f32 {
    if size.x <= 0.0 || size.y <= 0.0 {
        return 1.0;
    }
    let fit = (area.width() / size.x).min(area.height() / size.y);
    fit.min(1.0 / ppp.max(0.1)).max(MIN_SCALE)
}

/// A checkerboard behind transparent images, drawn only where the image is.
pub fn checkerboard(painter: &egui::Painter, rect: Rect, dark: bool) {
    let (a, b) = if dark {
        (egui::Color32::from_gray(40), egui::Color32::from_gray(52))
    } else {
        (egui::Color32::from_gray(220), egui::Color32::from_gray(238))
    };
    painter.rect_filled(rect, 0.0, a);
    let cell = 10.0;
    let clip = rect.intersect(painter.clip_rect());
    if clip.width() <= 0.0 || clip.height() <= 0.0 {
        return;
    }
    // Only the cells that are actually on screen.
    let x0 = ((clip.min.x - rect.min.x) / cell).floor() as i32;
    let y0 = ((clip.min.y - rect.min.y) / cell).floor() as i32;
    let x1 = ((clip.max.x - rect.min.x) / cell).ceil() as i32;
    let y1 = ((clip.max.y - rect.min.y) / cell).ceil() as i32;
    if (x1 - x0) * (y1 - y0) > 40_000 {
        return;
    }
    for y in y0..y1 {
        for x in x0..x1 {
            if (x + y) % 2 == 0 {
                continue;
            }
            let min = pos2(rect.min.x + x as f32 * cell, rect.min.y + y as f32 * cell);
            let cell_rect = Rect::from_min_size(min, vec2(cell, cell)).intersect(rect);
            painter.rect_filled(cell_rect, 0.0, b);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))
    }

    #[test]
    fn fit_never_enlarges_and_centres() {
        let view = View::default();
        let small = vec2(200.0, 100.0);
        assert_eq!(view.scale(area(), small, 1.0), 1.0);
        let r = view.image_rect(area(), small, 1.0);
        assert_eq!(r.center(), area().center());

        let big = vec2(1600.0, 600.0);
        assert_eq!(view.scale(area(), big, 1.0), 0.5);
    }

    #[test]
    fn zooming_keeps_the_point_under_the_cursor() {
        let mut view = View::default();
        let size = vec2(1600.0, 1200.0);
        let anchor = pos2(123.0, 456.0);
        let before = view.image_pos(area(), size, 1.0, anchor);
        view.zoom_by(area(), size, 1.0, 3.0, anchor);
        let after = view.image_pos(area(), size, 1.0, anchor);
        assert!((before - after).length() < 1e-3);
        // And back to screen gives the anchor again.
        let back = view.screen_pos(area(), size, 1.0, after);
        assert!((back - anchor).length() < 1e-3);
    }

    #[test]
    fn actual_size_respects_display_scaling() {
        let mut view = View::default();
        let size = vec2(4000.0, 3000.0);
        view.actual_size(area(), size, 1.5, area().center());
        assert!((view.percent(area(), size, 1.5) - 100.0).abs() < 1e-3);
    }
}
