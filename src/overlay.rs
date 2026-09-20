//! The selection overlay.
//!
//! The screen is captured first and the overlay then draws that frozen frame
//! full screen. Everything the user sees while selecting is a still image, so
//! a video, an animation or a tooltip cannot change between aiming and
//! clicking, and the region that gets saved is exactly the region that was
//! highlighted.

use egui::{
    Align2, Color32, CornerRadius, FontId, Pos2, Rect as UiRect, Sense, Stroke, Vec2, pos2,
};

use crate::config::Overlay as OverlayConfig;
use crate::platform::{Image, Rect, WindowInfo};

/// Movement below this many pixels counts as a click, not a drag.
const CLICK_SLOP: i32 = 4;
/// Half width, in source pixels, of the area the magnifier shows.
const MAGNIFIER_RADIUS: f32 = 11.0;
const MAGNIFIER_SIZE: f32 = 132.0;

pub enum Outcome {
    /// Still running.
    Pending,
    Cancelled,
    /// A rectangle in physical screen coordinates.
    Selected(Rect),
}

pub struct Overlay {
    pub image: Image,
    pub screen: Rect,
    windows: Vec<WindowInfo>,
    texture: Option<egui::TextureHandle>,
    drag_start: Option<(i32, i32)>,
    dragged: bool,
    cursor: (i32, i32),
    accent: Color32,
    config: OverlayConfig,
}

impl Overlay {
    pub fn new(
        image: Image,
        screen: Rect,
        windows: Vec<WindowInfo>,
        config: OverlayConfig,
        accent: [u8; 3],
    ) -> Self {
        Self {
            image,
            screen,
            windows,
            texture: None,
            drag_start: None,
            dragged: false,
            // The window under the mouse has to be right on the very first
            // frame. Waiting for egui to report a pointer position would mean
            // highlighting whatever happens to sit at the top left corner
            // until the user moves the mouse.
            cursor: crate::platform::cursor_position(),
            accent: Color32::from_rgb(accent[0], accent[1], accent[2]),
            config,
        }
    }

    /// The window under the cursor, or `None` when detection is off or the
    /// cursor sits on the desktop.
    fn window_under_cursor(&self) -> Option<&WindowInfo> {
        if !self.config.detect_windows {
            return None;
        }
        // The list is front to back, so the first hit is the visible one.
        self.windows
            .iter()
            .find(|w| w.rect.contains(self.cursor.0, self.cursor.1))
    }

    /// The top most window containing a point, for naming the saved file.
    pub fn window_at(&self, x: i32, y: i32) -> Option<&WindowInfo> {
        self.windows.iter().find(|w| w.rect.contains(x, y))
    }

    /// Draw one frame and report what the user did.
    pub fn ui(&mut self, ui: &mut egui::Ui) -> Outcome {
        let ctx = ui.ctx().clone();
        ctx.set_cursor_icon(egui::CursorIcon::Crosshair);

        let image = &self.image;
        let texture = self.texture.get_or_insert_with(|| {
            let color = egui::ColorImage::from_rgba_unmultiplied(
                [image.width as usize, image.height as usize],
                &image.rgba,
            );
            // Nearest keeps the magnifier showing real pixels rather than a
            // blurred average, and the full screen draw is 1:1 anyway.
            ctx.load_texture("visura-frozen-screen", color, egui::TextureOptions::NEAREST)
        });
        let texture_id = texture.id();

        let area = ui.max_rect();
        // Swallow clicks so nothing underneath reacts while selecting.
        ui.allocate_rect(area, Sense::click_and_drag());
        let painter = ui.painter().clone();

        // The window is asked to cover the whole desktop, but the window
        // manager needs a frame or two to comply. Until the size matches,
        // the frozen frame is shown and input is ignored, so an early click
        // cannot land on the wrong pixels.
        let points = ui.ctx().pixels_per_point();
        let settled = (area.width() * points - self.screen.w as f32).abs() <= 3.0
            && (area.height() * points - self.screen.h as f32).abs() <= 3.0;
        if !settled {
            painter.image(
                texture_id,
                area,
                UiRect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            return Outcome::Pending;
        }

        // The window is meant to cover the whole virtual desktop, but its real
        // size is whatever the window manager granted. Mapping from the actual
        // rectangle keeps the selection exact either way.
        let map = Mapping::new(area, self.screen);

        if let Some(p) = ctx.input(|i| i.pointer.latest_pos()) {
            self.cursor = map.to_physical(p);
        }

        // ------------------------------------------------------- input ----
        let (pressed, released, secondary, escape, space, enter) = ctx.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_released(),
                i.pointer.secondary_pressed(),
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::Space),
                i.key_pressed(egui::Key::Enter),
            )
        });

        if escape || secondary {
            return Outcome::Cancelled;
        }
        if space {
            return Outcome::Selected(self.screen);
        }

        if pressed {
            self.drag_start = Some(self.cursor);
            self.dragged = false;
        }
        if let Some(start) = self.drag_start
            && ((self.cursor.0 - start.0).abs() > CLICK_SLOP
                || (self.cursor.1 - start.1).abs() > CLICK_SLOP)
        {
            self.dragged = true;
        }

        let selection = self
            .drag_start
            .filter(|_| self.dragged)
            .map(|start| Rect::from_corners(start, self.cursor));

        if released {
            let start = self.drag_start.take();
            if self.dragged {
                if let Some(start) = start {
                    let region = Rect::from_corners(start, self.cursor).intersect(&self.screen);
                    self.dragged = false;
                    if region.w >= 2 && region.h >= 2 {
                        return Outcome::Selected(region);
                    }
                }
            } else if let Some(window) = self.window_under_cursor() {
                // A plain click takes the highlighted window.
                return Outcome::Selected(window.rect);
            }
            self.dragged = false;
        }

        let hovered = if selection.is_none() {
            self.window_under_cursor().cloned()
        } else {
            None
        };
        if enter && let Some(region) = selection.or_else(|| hovered.as_ref().map(|w| w.rect)) {
            return Outcome::Selected(region);
        }

        // ------------------------------------------------------- paint ----
        painter.image(
            texture_id,
            area,
            UiRect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );

        let dim = Color32::from_black_alpha((self.config.dim * 255.0) as u8);
        match (selection, &hovered) {
            // A region being dragged is cut out of the dimming, so the part
            // that will be saved keeps its real colours while it is chosen.
            (Some(region), _) => {
                let hole = map.to_points(region);
                for band in [
                    UiRect::from_min_max(area.min, pos2(area.max.x, hole.min.y)),
                    UiRect::from_min_max(pos2(area.min.x, hole.max.y), area.max),
                    UiRect::from_min_max(
                        pos2(area.min.x, hole.min.y),
                        pos2(hole.min.x, hole.max.y),
                    ),
                    UiRect::from_min_max(
                        pos2(hole.max.x, hole.min.y),
                        pos2(area.max.x, hole.max.y),
                    ),
                ] {
                    if band.is_positive() {
                        painter.rect_filled(band, 0.0, dim);
                    }
                }
                painter.rect_stroke(
                    hole,
                    0.0,
                    Stroke::new(2.0, self.accent),
                    egui::StrokeKind::Outside,
                );
                self.draw_handles(&painter, hole);
                self.draw_readout(&painter, hole, region, None);
            }
            // A detected window is outlined instead. Cutting it out of the
            // dimming made it hard to tell where the edge actually ran; a line
            // drawn on the edge says exactly what a click would take.
            (None, Some(window)) => {
                painter.rect_filled(area, 0.0, dim);
                let outline = map.to_points(window.rect);
                painter.rect_stroke(
                    outline,
                    0.0,
                    Stroke::new(2.0, self.accent),
                    egui::StrokeKind::Inside,
                );
                // A thin dark line just outside keeps the accent readable on a
                // light background as well as a dark one.
                painter.rect_stroke(
                    outline.expand(1.0),
                    0.0,
                    Stroke::new(1.0, Color32::from_black_alpha(140)),
                    egui::StrokeKind::Inside,
                );
                self.draw_readout(&painter, outline, window.rect, Some(window));
            }
            (None, None) => {
                painter.rect_filled(area, 0.0, dim);
            }
        }

        if self.config.crosshair && selection.is_none() {
            let p = map.to_ui(self.cursor);
            let line = Stroke::new(1.0, Color32::from_white_alpha(70));
            painter.line_segment([pos2(area.min.x, p.y), pos2(area.max.x, p.y)], line);
            painter.line_segment([pos2(p.x, area.min.y), pos2(p.x, area.max.y)], line);
        }

        if self.config.magnifier {
            self.draw_magnifier(&painter, texture_id, area, &map);
        }

        if self.config.show_hints && selection.is_none() {
            self.draw_hints(&painter, area);
        }

        Outcome::Pending
    }

    fn draw_handles(&self, painter: &egui::Painter, hole: UiRect) {
        let arm = 14.0_f32.min(hole.width() / 3.0).min(hole.height() / 3.0);
        if arm < 4.0 {
            return;
        }
        let stroke = Stroke::new(2.0, self.accent);
        for (corner, dx, dy) in [
            (hole.left_top(), 1.0, 1.0),
            (hole.right_top(), -1.0, 1.0),
            (hole.left_bottom(), 1.0, -1.0),
            (hole.right_bottom(), -1.0, -1.0),
        ] {
            painter.line_segment([corner, corner + Vec2::new(arm * dx, 0.0)], stroke);
            painter.line_segment([corner, corner + Vec2::new(0.0, arm * dy)], stroke);
        }
    }

    /// The size, or the window title when a window is highlighted.
    fn draw_readout(
        &self,
        painter: &egui::Painter,
        hole: UiRect,
        region: Rect,
        window: Option<&WindowInfo>,
    ) {
        let text = match window {
            Some(w) => format!("{}  ·  {} × {}", trim(&w.title, 48), region.w, region.h),
            None => format!("{} × {}", region.w, region.h),
        };

        // Above the selection, unless there is no room up there.
        let anchor = if hole.min.y > 30.0 {
            pos2(hole.min.x, hole.min.y - 6.0)
        } else {
            pos2(hole.min.x, hole.max.y + 6.0)
        };
        let align = if hole.min.y > 30.0 {
            Align2::LEFT_BOTTOM
        } else {
            Align2::LEFT_TOP
        };

        let font = FontId::proportional(13.0);
        let galley = painter.layout_no_wrap(text, font, Color32::WHITE);
        let rect = align.anchor_size(anchor, galley.size() + Vec2::new(14.0, 8.0));
        painter.rect_filled(rect, CornerRadius::same(4), Color32::from_black_alpha(200));
        painter.galley(rect.min + Vec2::new(7.0, 4.0), galley, Color32::WHITE);
    }

    fn draw_magnifier(
        &self,
        painter: &egui::Painter,
        texture_id: egui::TextureId,
        area: UiRect,
        map: &Mapping,
    ) {
        let cursor = map.to_ui(self.cursor);
        // Keep the magnifier off the cursor and inside the screen.
        let mut origin = cursor + Vec2::new(22.0, 22.0);
        if origin.x + MAGNIFIER_SIZE > area.max.x - 8.0 {
            origin.x = cursor.x - MAGNIFIER_SIZE - 22.0;
        }
        if origin.y + MAGNIFIER_SIZE + 26.0 > area.max.y - 8.0 {
            origin.y = cursor.y - MAGNIFIER_SIZE - 48.0;
        }
        let box_rect = UiRect::from_min_size(origin, Vec2::splat(MAGNIFIER_SIZE));

        let (w, h) = (self.image.width as f32, self.image.height as f32);
        let cx = (self.cursor.0 - self.screen.x) as f32;
        let cy = (self.cursor.1 - self.screen.y) as f32;
        let uv = UiRect::from_min_max(
            pos2((cx - MAGNIFIER_RADIUS) / w, (cy - MAGNIFIER_RADIUS) / h),
            pos2(
                (cx + MAGNIFIER_RADIUS + 1.0) / w,
                (cy + MAGNIFIER_RADIUS + 1.0) / h,
            ),
        );

        painter.rect_filled(box_rect, CornerRadius::same(3), Color32::BLACK);
        painter.image(texture_id, box_rect, uv, Color32::WHITE);
        painter.rect_stroke(
            box_rect,
            CornerRadius::same(3),
            Stroke::new(1.0, Color32::from_white_alpha(60)),
            egui::StrokeKind::Outside,
        );

        // Mark the pixel that is actually under the cursor.
        let cell = MAGNIFIER_SIZE / (MAGNIFIER_RADIUS * 2.0 + 1.0);
        let centre = UiRect::from_min_size(
            box_rect.min + Vec2::splat(cell * MAGNIFIER_RADIUS),
            Vec2::splat(cell),
        );
        painter.rect_stroke(
            centre,
            0.0,
            Stroke::new(1.0, self.accent),
            egui::StrokeKind::Outside,
        );

        let px = self
            .image
            .pixel((cx as i32).max(0) as u32, (cy as i32).max(0) as u32);
        let label = format!(
            "{}, {}   #{:02X}{:02X}{:02X}",
            self.cursor.0, self.cursor.1, px[0], px[1], px[2]
        );
        let strip = UiRect::from_min_size(
            pos2(box_rect.min.x, box_rect.max.y + 4.0),
            Vec2::new(MAGNIFIER_SIZE, 20.0),
        );
        painter.rect_filled(strip, CornerRadius::same(3), Color32::from_black_alpha(200));
        painter.text(
            strip.center(),
            Align2::CENTER_CENTER,
            label,
            FontId::monospace(11.0),
            Color32::from_white_alpha(220),
        );
    }

    fn draw_hints(&self, painter: &egui::Painter, area: UiRect) {
        let hint = if self.config.detect_windows {
            "Drag: region   ·   Click: window   ·   Space: whole screen   ·   Esc: cancel"
        } else {
            "Drag: region   ·   Space: whole screen   ·   Esc: cancel"
        };
        let font = FontId::proportional(13.0);
        let galley = painter.layout_no_wrap(hint.to_string(), font, Color32::from_white_alpha(210));
        let rect = Align2::CENTER_BOTTOM.anchor_size(
            pos2(area.center().x, area.max.y - 36.0),
            galley.size() + Vec2::new(24.0, 14.0),
        );
        painter.rect_filled(rect, CornerRadius::same(6), Color32::from_black_alpha(190));
        painter.galley(
            rect.min + Vec2::new(12.0, 7.0),
            galley,
            Color32::from_white_alpha(210),
        );
    }
}

fn trim(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
}

/// Converts between the overlay window and physical screen pixels.
///
/// Both axes are scaled independently from the real window rectangle, so a
/// window manager that resizes or offsets the overlay slightly cannot shift
/// the saved region.
struct Mapping {
    area: UiRect,
    screen: Rect,
}

impl Mapping {
    fn new(area: UiRect, screen: Rect) -> Self {
        Self { area, screen }
    }

    fn to_physical(&self, p: Pos2) -> (i32, i32) {
        let fx = ((p.x - self.area.min.x) / self.area.width().max(1.0)).clamp(0.0, 1.0);
        let fy = ((p.y - self.area.min.y) / self.area.height().max(1.0)).clamp(0.0, 1.0);
        (
            self.screen.x + (fx * self.screen.w as f32).round() as i32,
            self.screen.y + (fy * self.screen.h as f32).round() as i32,
        )
    }

    fn to_ui(&self, p: (i32, i32)) -> Pos2 {
        let fx = (p.0 - self.screen.x) as f32 / self.screen.w.max(1) as f32;
        let fy = (p.1 - self.screen.y) as f32 / self.screen.h.max(1) as f32;
        pos2(
            self.area.min.x + fx * self.area.width(),
            self.area.min.y + fy * self.area.height(),
        )
    }

    fn to_points(&self, r: Rect) -> UiRect {
        UiRect::from_min_max(self.to_ui((r.x, r.y)), self.to_ui((r.right(), r.bottom())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mapping() -> Mapping {
        Mapping::new(
            UiRect::from_min_size(pos2(0.0, 0.0), Vec2::new(1720.0, 720.0)),
            Rect::new(-1920, 0, 3440, 1440),
        )
    }

    #[test]
    fn the_corners_map_onto_the_screen_corners() {
        let m = mapping();
        assert_eq!(m.to_physical(pos2(0.0, 0.0)), (-1920, 0));
        assert_eq!(m.to_physical(pos2(1720.0, 720.0)), (1520, 1440));
    }

    #[test]
    fn mapping_back_and_forth_is_stable() {
        let m = mapping();
        for point in [(-1920, 0), (0, 700), (1519, 1439)] {
            assert_eq!(m.to_physical(m.to_ui(point)), point);
        }
    }

    #[test]
    fn a_pointer_outside_the_window_is_clamped() {
        let m = mapping();
        assert_eq!(m.to_physical(pos2(-50.0, -50.0)), (-1920, 0));
        assert_eq!(m.to_physical(pos2(9000.0, 9000.0)), (1520, 1440));
    }

    #[test]
    fn corners_produce_a_positive_rectangle_in_any_drag_direction() {
        let a = Rect::from_corners((100, 80), (40, 10));
        assert_eq!(a, Rect::new(40, 10, 60, 70));
        let b = Rect::from_corners((40, 10), (100, 80));
        assert_eq!(a, b);
    }

    #[test]
    fn long_titles_are_shortened_with_an_ellipsis() {
        assert_eq!(trim("abcdef", 4), "abc…");
        assert_eq!(trim("abc", 4), "abc");
    }
}
