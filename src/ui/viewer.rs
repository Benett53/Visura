//! A plain image viewer: one screenshot, zoom and pan, previous and next.

use std::path::{Path, PathBuf};

use egui::{Align, Key, Layout, RichText, Sense, TextureOptions, Vec2};

use super::canvas::{View, WHEEL_STEP};
use super::theme::Palette;

/// What the viewer asks the application to do.
pub enum Action {
    Close,
    Edit(PathBuf),
    /// Show the shot this many places further in the library.
    Step(i32),
    Copy(PathBuf),
    OpenExternally(PathBuf),
}

pub struct Viewer {
    pub path: PathBuf,
    image: Result<Loaded, String>,
    view: View,
}

pub struct Loaded {
    pub texture: egui::TextureHandle,
    pub size: Vec2,
}

/// Decode a file into a texture that stays sharp when zoomed in.
pub fn load_texture(ctx: &egui::Context, path: &Path) -> Result<Loaded, String> {
    let decoded = image::open(path)
        .map_err(|e| format!("{} could not be read: {e}", path.display()))?
        .to_rgba8();
    let size = [decoded.width() as usize, decoded.height() as usize];
    let pixels = egui::ColorImage::from_rgba_unmultiplied(size, decoded.as_raw());
    let texture = ctx.load_texture(
        format!("visura-view-{}", path.display()),
        pixels,
        TextureOptions {
            magnification: egui::TextureFilter::Nearest,
            minification: egui::TextureFilter::Linear,
            ..TextureOptions::LINEAR
        },
    );
    Ok(Loaded {
        texture,
        size: egui::vec2(size[0] as f32, size[1] as f32),
    })
}

impl Viewer {
    pub fn open(ctx: &egui::Context, path: PathBuf) -> Self {
        Self {
            image: load_texture(ctx, &path),
            path,
            view: View::default(),
        }
    }

    pub fn title(&self) -> String {
        let name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        format!("{name} - Visura")
    }

    /// `position` is this shot's place in the library and the library size,
    /// for the counter and for greying out the arrows at either end.
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        palette: &Palette,
        position: Option<(usize, usize)>,
    ) -> Vec<Action> {
        let mut actions = Vec::new();
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();

        // Keys first, so they work wherever the mouse is.
        ctx.input(|i| {
            if i.key_pressed(Key::Escape) {
                actions.push(Action::Close);
            }
            if i.key_pressed(Key::ArrowLeft) {
                actions.push(Action::Step(-1));
            }
            if i.key_pressed(Key::ArrowRight) {
                actions.push(Action::Step(1));
            }
            if i.key_pressed(Key::E) && !i.modifiers.any() {
                actions.push(Action::Edit(self.path.clone()));
            }
            // egui may report Ctrl+C as a copy event, a key press or both.
            let copy = (i.modifiers.command && i.key_pressed(Key::C))
                || i.events.iter().any(|e| matches!(e, egui::Event::Copy));
            if copy {
                actions.push(Action::Copy(self.path.clone()));
            }
        });

        egui::Panel::top(egui::Id::new("visura-viewer-bar"))
            .exact_size(40.0)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let (index, total) = position.unwrap_or((0, 1));
                    if ui
                        .add_enabled(index > 0, egui::Button::new("‹"))
                        .on_hover_text("Previous  (←)")
                        .clicked()
                    {
                        actions.push(Action::Step(-1));
                    }
                    if ui
                        .add_enabled(index + 1 < total, egui::Button::new("›"))
                        .on_hover_text("Next  (→)")
                        .clicked()
                    {
                        actions.push(Action::Step(1));
                    }
                    if position.is_some() {
                        ui.label(
                            RichText::new(format!("{} / {}", index + 1, total))
                                .color(palette.muted),
                        );
                    }
                    ui.separator();
                    let name = self
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    ui.label(RichText::new(name).color(palette.text));

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Edit").on_hover_text("E").clicked() {
                            actions.push(Action::Edit(self.path.clone()));
                        }
                        if ui.button("Copy").on_hover_text("Ctrl+C").clicked() {
                            actions.push(Action::Copy(self.path.clone()));
                        }
                        if ui
                            .button("Open with …")
                            .on_hover_text("Open in the default program")
                            .clicked()
                        {
                            actions.push(Action::OpenExternally(self.path.clone()));
                        }
                        ui.separator();
                        if let Ok(loaded) = &self.image {
                            let area = ui.ctx().content_rect();
                            if ui.button("100 %").on_hover_text("1").clicked() {
                                self.view.actual_size(area, loaded.size, ppp, area.center());
                            }
                            if ui
                                .selectable_label(self.view.is_fit(), "Fit")
                                .on_hover_text("0")
                                .clicked()
                            {
                                self.view.fit();
                            }
                        }
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(palette.background))
            .show(ui, |ui| {
                let (area, response) =
                    ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());
                let loaded = match &self.image {
                    Ok(loaded) => loaded,
                    Err(e) => {
                        ui.painter().text(
                            area.center(),
                            egui::Align2::CENTER_CENTER,
                            e,
                            egui::FontId::proportional(14.0),
                            palette.danger,
                        );
                        return;
                    }
                };
                let size = loaded.size;
                navigate(ui, &response, &mut self.view, area, size, ppp, true);

                let painter = ui.painter_at(area);
                let rect = self.view.image_rect(area, size, ppp);
                super::canvas::checkerboard(&painter, rect, palette.background.r() < 128);
                painter.image(
                    loaded.texture.id(),
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );

                let percent = self.view.percent(area, size, ppp);
                let info = format!("{} × {}   ·   {:.0} %", size.x, size.y, percent);
                let galley =
                    painter.layout_no_wrap(info, egui::FontId::proportional(12.0), palette.muted);
                let at = area.right_bottom() - galley.size() - egui::vec2(10.0, 8.0);
                painter.rect_filled(
                    egui::Rect::from_min_size(at, galley.size()).expand(4.0),
                    4.0,
                    palette.panel.gamma_multiply(0.85),
                );
                painter.galley(at, galley, palette.muted);
            });

        actions
    }
}

/// Wheel and keys zoom, dragging pans, a double click switches between fit
/// and actual size. Shared with the editor, where the left button belongs to
/// the tools: there only the middle button pans and a double click does
/// nothing here.
pub fn navigate(
    ui: &egui::Ui,
    response: &egui::Response,
    view: &mut View,
    area: egui::Rect,
    size: Vec2,
    ppp: f32,
    primary_pans: bool,
) {
    let pointer = response.hover_pos().unwrap_or(area.center());
    let (notches, keys) = ui.input(|i| {
        // The viewer takes plain keys; in the editor they pick tools and
        // colours, so zooming there wants Ctrl.
        let wanted = i.modifiers.command != primary_pans;
        let pressed = |key| wanted && i.key_pressed(key);
        let keys = (
            pressed(Key::Plus) || pressed(Key::Equals),
            pressed(Key::Minus),
            pressed(Key::Num0),
            pressed(Key::Num1),
        );
        // Wheel notches straight from the events: egui's own scroll delta is
        // smoothed over several frames and turns Ctrl+wheel into a zoom of
        // its own.
        let notches: f32 = i
            .events
            .iter()
            .map(|e| match e {
                egui::Event::MouseWheel { unit, delta, .. } => match unit {
                    egui::MouseWheelUnit::Point => delta.y / 50.0,
                    egui::MouseWheelUnit::Line => delta.y,
                    egui::MouseWheelUnit::Page => delta.y * 10.0,
                },
                _ => 0.0,
            })
            .sum();
        (notches, keys)
    });
    if response.hovered() && notches != 0.0 {
        view.zoom_by(area, size, ppp, WHEEL_STEP.powf(notches), pointer);
    }
    let (zoom_in, zoom_out, fit, actual) = keys;
    if zoom_in {
        view.zoom_by(area, size, ppp, WHEEL_STEP, area.center());
    }
    if zoom_out {
        view.zoom_by(area, size, ppp, 1.0 / WHEEL_STEP, area.center());
    }
    if fit {
        view.fit();
    }
    if actual {
        view.actual_size(area, size, ppp, area.center());
    }
    if (primary_pans && response.dragged_by(egui::PointerButton::Primary))
        || response.dragged_by(egui::PointerButton::Middle)
    {
        view.pan(area, size, ppp, response.drag_delta());
    }
    if primary_pans && response.double_clicked() {
        if view.is_fit() {
            view.actual_size(area, size, ppp, pointer);
        } else {
            view.fit();
        }
    }
}
