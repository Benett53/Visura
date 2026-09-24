//! The image editor: annotations, redaction and cropping on top of a shot.
//!
//! Everything is kept as a list of annotations over the untouched image (see
//! `model`), shown with egui on screen and burnt into the pixels only when the
//! result is saved or copied (see `render`).

pub mod effects;
pub mod model;
pub mod render;

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use egui::{
    Align, Color32, CursorIcon, Event, FontId, Key, Layout, Pos2, Rect, RichText, Sense, Stroke,
    TextureHandle, TextureOptions, Vec2, pos2, vec2,
};

use crate::config::Format;
use crate::platform::Image;
use crate::ui::canvas::View;
use crate::ui::theme::Palette;
use model::{Annotation, Doc, History, Shape, Style, Tool};
use render::Mapping;

/// What the editor asks the application to do.
pub enum Action {
    Close,
    /// A file was written; the library should pick it up.
    Saved(PathBuf),
    Note(String),
    Warn(String),
}

/// Colours offered with one click, and on the number keys 1 to 8.
pub const SWATCHES: [Color32; 8] = [
    Color32::from_rgb(0xe5, 0x39, 0x35),
    Color32::from_rgb(0xfb, 0x8c, 0x00),
    Color32::from_rgb(0xfd, 0xd8, 0x35),
    Color32::from_rgb(0x43, 0xa0, 0x47),
    Color32::from_rgb(0x1e, 0x88, 0xe5),
    Color32::from_rgb(0x8e, 0x24, 0xaa),
    Color32::WHITE,
    Color32::BLACK,
];

/// Everything the editor remembers between two edits in one session: the
/// last tool and its settings.
#[derive(Clone, Copy)]
pub struct Settings {
    pub tool: Tool,
    pub style: Style,
    pub blur_radius: f32,
    pub pixel_size: f32,
    pub spot_dim: f32,
    pub spot_round: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            tool: Tool::Rectangle,
            style: Style::default(),
            blur_radius: 10.0,
            pixel_size: 12.0,
            spot_dim: 0.6,
            spot_round: false,
        }
    }
}

enum Gesture {
    /// Drawing the last annotation in the list.
    Create {
        start: Pos2,
        before: Doc,
    },
    Move {
        index: usize,
        last: Pos2,
        before: Doc,
        moved: bool,
    },
    Resize {
        index: usize,
        handle: Handle,
        before: Doc,
    },
    Crop {
        start: Pos2,
    },
    Pan,
}

#[derive(Clone, Copy, PartialEq)]
enum Handle {
    /// Corner of a rectangle: 0 top left, 1 top right, 2 bottom right,
    /// 3 bottom left.
    Corner(usize),
    Start,
    End,
}

pub struct Editor {
    pub path: PathBuf,
    base: Image,
    texture: TextureHandle,
    doc: Doc,
    saved: Doc,
    history: History,
    pub settings: Settings,
    view: View,
    region: Rect,
    gesture: Option<Gesture>,
    selected: Option<usize>,
    /// The text annotation being typed into.
    editing: Option<usize>,
    crop_draft: Option<Rect>,
    /// Measured size of each text annotation, from the last frame.
    text_sizes: HashMap<usize, Vec2>,
    patches: HashMap<u64, (TextureHandle, Rect)>,
    spotlight: Option<(u64, TextureHandle)>,
    /// The item whose style is being changed, once that change is recorded
    /// for undo; further tweaks of the same item join that one step.
    restyling: Option<usize>,
    fonts: Option<egui::epaint::Fonts>,
    jpeg_quality: u8,
    confirm_close: bool,
    show_keys: bool,
    /// A short message in the status bar: text, error or not, and since when.
    message: Option<(String, bool, std::time::Instant)>,
}

fn hash_of(value: impl Hash) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut h);
    h.finish()
}

fn rect_bits(r: Rect) -> [u32; 4] {
    [r.min.x, r.min.y, r.max.x, r.max.y].map(f32::to_bits)
}

/// The tool that makes a given kind of annotation, for its settings.
fn tool_of(shape: &Shape) -> Tool {
    match shape {
        Shape::Rectangle(_) => Tool::Rectangle,
        Shape::Ellipse(_) => Tool::Ellipse,
        Shape::Arrow(..) => Tool::Arrow,
        Shape::Line(..) => Tool::Line,
        Shape::Pen(_) => Tool::Pen,
        Shape::Highlighter(_) => Tool::Highlighter,
        Shape::Text(..) => Tool::Text,
        Shape::Counter(..) => Tool::Counter,
        Shape::Spotlight(_) => Tool::Spotlight,
        Shape::Blur(_) => Tool::Blur,
        Shape::Pixelate(_) => Tool::Pixelate,
        Shape::Redact(_) => Tool::Redact,
    }
}

/// Where a copy of `path` goes: `name_edited.png`, then `name_edited_2.png`
/// and so on. Formats Visura cannot write become PNG.
pub fn copy_path(path: &Path) -> PathBuf {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "shot".into());
    let ext = match writable_format(path) {
        Some(Format::Jpeg) => "jpg",
        _ => "png",
    };
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut n = 1;
    loop {
        let name = if n == 1 {
            format!("{stem}_edited.{ext}")
        } else {
            format!("{stem}_edited_{n}.{ext}")
        };
        let candidate = dir.join(name);
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}

fn writable_format(path: &Path) -> Option<Format> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => Some(Format::Png),
        Some("jpg" | "jpeg") => Some(Format::Jpeg),
        _ => None,
    }
}

impl Editor {
    pub fn open(
        ctx: &egui::Context,
        path: PathBuf,
        settings: Settings,
        jpeg_quality: u8,
    ) -> Result<Self, String> {
        let decoded = image::open(&path)
            .map_err(|e| format!("{} could not be read: {e}", path.display()))?
            .to_rgba8();
        let (w, h) = (decoded.width(), decoded.height());
        let texture = ctx.load_texture(
            format!("visura-edit-{}", path.display()),
            egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], decoded.as_raw()),
            TextureOptions {
                magnification: egui::TextureFilter::Nearest,
                minification: egui::TextureFilter::Linear,
                ..TextureOptions::LINEAR
            },
        );
        let full = Rect::from_min_size(Pos2::ZERO, vec2(w as f32, h as f32));
        Ok(Self {
            path,
            base: Image {
                width: w,
                height: h,
                rgba: decoded.into_raw(),
            },
            texture,
            doc: Doc::default(),
            saved: Doc::default(),
            history: History::default(),
            settings,
            view: View::default(),
            region: full,
            gesture: None,
            selected: None,
            editing: None,
            crop_draft: None,
            text_sizes: HashMap::new(),
            patches: HashMap::new(),
            spotlight: None,
            restyling: None,
            fonts: None,
            jpeg_quality,
            confirm_close: false,
            show_keys: false,
            message: None,
        })
    }

    /// Show a message in the status bar for a few seconds.
    pub fn flash(&mut self, text: String, error: bool) {
        self.message = Some((text, error, std::time::Instant::now()));
    }

    pub fn is_dirty(&self) -> bool {
        self.doc != self.saved
    }

    pub fn title(&self) -> String {
        let name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mark = if self.is_dirty() { "• " } else { "" };
        format!("{mark}{name} - Visura editor")
    }

    /// Ask to close; with unsaved changes the user is asked first.
    pub fn request_close(&mut self, actions: &mut Vec<Action>) {
        if self.is_dirty() {
            self.confirm_close = true;
        } else {
            actions.push(Action::Close);
        }
    }

    fn full(&self) -> Rect {
        Rect::from_min_size(
            Pos2::ZERO,
            vec2(self.base.width as f32, self.base.height as f32),
        )
    }

    // ------------------------------------------------------------ styles --

    /// The style a new annotation of this kind gets from the settings.
    fn style_for(&self, tool: Tool) -> Style {
        let s = &self.settings;
        Style {
            strength: match tool {
                Tool::Blur => s.blur_radius,
                Tool::Pixelate => s.pixel_size,
                Tool::Spotlight => s.spot_dim,
                _ => 0.0,
            },
            round: s.spot_round,
            ..s.style
        }
    }

    /// Show a selected annotation's style in the settings, so it can be
    /// changed there.
    fn load_style(&mut self, index: usize) {
        let Some(item) = self.doc.items.get(index) else {
            return;
        };
        let (tool, style) = (tool_of(&item.shape), item.style);
        let s = &mut self.settings;
        s.style.color = style.color;
        s.style.fill = style.fill;
        s.style.width = style.width;
        s.style.font_size = style.font_size;
        match tool {
            Tool::Blur => s.blur_radius = style.strength,
            Tool::Pixelate => s.pixel_size = style.strength,
            Tool::Spotlight => {
                s.spot_dim = style.strength;
                s.spot_round = style.round;
            }
            _ => {}
        }
    }

    fn select(&mut self, index: Option<usize>) {
        if self.selected != index {
            self.restyling = None;
        }
        self.selected = index;
        if let Some(i) = index {
            self.load_style(i);
        }
    }

    /// After the settings changed: restyle the selected annotation.
    fn apply_settings_to_selection(&mut self) {
        let Some(index) = self.selected else {
            return;
        };
        let Some(item) = self.doc.items.get(index) else {
            return;
        };
        let style = self.style_for(tool_of(&item.shape));
        if item.style == style {
            return;
        }
        if self.restyling != Some(index) {
            self.history.record(&self.doc);
            self.restyling = Some(index);
        }
        self.doc.items[index].style = style;
    }

    // ------------------------------------------------------------ editing --

    fn set_tool(&mut self, tool: Tool) {
        self.finish_text();
        if tool != Tool::Select {
            self.select(None);
        }
        if tool == Tool::Crop {
            self.crop_draft = self.doc.crop;
        } else {
            self.crop_draft = None;
        }
        self.settings.tool = tool;
    }

    fn undo(&mut self) {
        self.finish_text();
        if self.history.undo(&mut self.doc) {
            self.after_history();
        }
    }

    fn redo(&mut self) {
        self.finish_text();
        if self.history.redo(&mut self.doc) {
            self.after_history();
        }
    }

    fn after_history(&mut self) {
        self.selected = None;
        self.restyling = None;
        self.gesture = None;
        if self.settings.tool == Tool::Crop {
            self.crop_draft = self.doc.crop;
        }
    }

    fn delete_selected(&mut self) {
        if let Some(index) = self.selected.take() {
            if index < self.doc.items.len() {
                self.history.record(&self.doc);
                self.doc.items.remove(index);
            }
            self.restyling = None;
        }
    }

    /// Stop typing. Text that ended up empty is removed again.
    fn finish_text(&mut self) {
        let Some(index) = self.editing.take() else {
            return;
        };
        if let Some(Annotation {
            shape: Shape::Text(_, text),
            ..
        }) = self.doc.items.get(index)
            && text.trim().is_empty()
        {
            self.doc.items.remove(index);
            if self.selected == Some(index) {
                self.selected = None;
            }
        }
    }

    fn apply_crop(&mut self) {
        let Some(draft) = self.crop_draft else {
            return;
        };
        let draft = draft.intersect(self.full());
        let rounded = Rect::from_min_max(
            pos2(draft.min.x.round(), draft.min.y.round()),
            pos2(draft.max.x.round(), draft.max.y.round()),
        );
        if rounded.width() < 2.0 || rounded.height() < 2.0 {
            return;
        }
        let crop = if rounded == self.full() {
            None
        } else {
            Some(rounded)
        };
        if crop != self.doc.crop {
            self.history.record(&self.doc);
            self.doc.crop = crop;
        }
        self.crop_draft = None;
        self.settings.tool = Tool::Select;
    }

    fn reset_crop(&mut self) {
        if self.doc.crop.is_some() {
            self.history.record(&self.doc);
            self.doc.crop = None;
        }
        self.crop_draft = None;
    }

    // ------------------------------------------------------------- output --

    fn render(&mut self) -> Image {
        self.finish_text();
        let fonts = self.fonts.get_or_insert_with(render::export_fonts);
        render::export(&self.base, &self.doc, fonts)
    }

    fn save(&mut self, as_copy: bool, actions: &mut Vec<Action>) -> bool {
        let format = writable_format(&self.path);
        let target = if as_copy || format.is_none() {
            copy_path(&self.path)
        } else {
            self.path.clone()
        };
        let format = writable_format(&target).unwrap_or(Format::Png);
        let image = self.render();
        let written = crate::capture::encode(&image, format, self.jpeg_quality).and_then(|bytes| {
            std::fs::write(&target, bytes)
                .map_err(|e| format!("{} could not be written: {e}", target.display()))
        });
        match written {
            Ok(()) => {
                // From now on the editor works on the file just written.
                self.path = target.clone();
                self.saved = self.doc.clone();
                let name = target
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                actions.push(Action::Saved(target));
                actions.push(Action::Note(format!("Saved {name}")));
                true
            }
            Err(e) => {
                actions.push(Action::Warn(e));
                false
            }
        }
    }

    fn copy(&mut self, actions: &mut Vec<Action>) {
        let image = self.render();
        match crate::clipboard::copy_image(&image) {
            Ok(()) => actions.push(Action::Note("Image copied".into())),
            Err(e) => actions.push(Action::Warn(format!("Copying failed: {e}"))),
        }
    }

    // ----------------------------------------------------------------- ui --

    pub fn ui(&mut self, ui: &mut egui::Ui, palette: &Palette) -> Vec<Action> {
        let mut actions = Vec::new();
        let ctx = ui.ctx().clone();

        self.keyboard(&ctx, &mut actions);

        egui::Panel::top(egui::Id::new("visura-editor-top"))
            .exact_size(44.0)
            .show(ui, |ui| self.top_bar(ui, palette, &mut actions));
        egui::Panel::bottom(egui::Id::new("visura-editor-status"))
            .exact_size(26.0)
            .show(ui, |ui| self.status_bar(ui, palette));
        egui::Panel::left(egui::Id::new("visura-editor-tools"))
            .exact_size(150.0)
            .resizable(false)
            .show(ui, |ui| self.tool_list(ui, palette));
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(palette.background))
            .show(ui, |ui| self.canvas(ui, palette));

        self.close_dialog(&ctx, &mut actions);
        self.keys_window(&ctx);
        actions
    }

    fn keyboard(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        if self.confirm_close {
            return;
        }
        // A settings field that has the keyboard gets the keys.
        let field_focused = ctx.memory(|m| m.focused().is_some());

        if let Some(index) = self.editing {
            let mut done = false;
            ctx.input(|i| {
                for event in &i.events {
                    match event {
                        Event::Text(t) => self.type_text(index, |s| s.push_str(t)),
                        Event::Paste(t) => self.type_text(index, |s| s.push_str(t)),
                        Event::Key {
                            key,
                            pressed: true,
                            modifiers,
                            ..
                        } => match key {
                            Key::Backspace => self.type_text(index, |s| {
                                s.pop();
                            }),
                            Key::Enter if modifiers.command => done = true,
                            Key::Enter => self.type_text(index, |s| s.push('\n')),
                            Key::Escape => done = true,
                            _ => {}
                        },
                        _ => {}
                    }
                }
            });
            if done {
                self.finish_text();
            }
            return;
        }
        if field_focused {
            return;
        }

        let mut tool = None;
        let (mut undo, mut redo, mut delete, mut save, mut save_copy, mut copy) =
            (false, false, false, false, false, false);
        let (mut escape, mut enter, mut help) = (false, false, false);
        let mut width = 0.0;
        let mut swatch = None;
        ctx.input(|i| {
            let m = i.modifiers;
            for t in Tool::ALL {
                if !m.any() && i.key_pressed(t.key()) {
                    tool = Some(t);
                }
            }
            undo = m.command && !m.shift && i.key_pressed(Key::Z);
            redo = m.command && (i.key_pressed(Key::Y) || (m.shift && i.key_pressed(Key::Z)));
            delete = i.key_pressed(Key::Delete) || i.key_pressed(Key::Backspace);
            save = m.command && !m.shift && i.key_pressed(Key::S);
            save_copy = m.command && m.shift && i.key_pressed(Key::S);
            copy = (m.command && i.key_pressed(Key::C))
                || i.events.iter().any(|e| matches!(e, Event::Copy));
            escape = i.key_pressed(Key::Escape);
            enter = i.key_pressed(Key::Enter);
            help = i.key_pressed(Key::F1);
            if i.key_pressed(Key::OpenBracket) {
                width -= 1.0;
            }
            if i.key_pressed(Key::CloseBracket) {
                width += 1.0;
            }
            let digits = [
                Key::Num1,
                Key::Num2,
                Key::Num3,
                Key::Num4,
                Key::Num5,
                Key::Num6,
                Key::Num7,
                Key::Num8,
            ];
            for (n, key) in digits.iter().enumerate() {
                if !m.any() && i.key_pressed(*key) {
                    swatch = Some(n);
                }
            }
        });

        if let Some(t) = tool {
            self.set_tool(t);
        }
        if undo {
            self.undo();
        }
        if redo {
            self.redo();
        }
        if delete {
            self.delete_selected();
        }
        if save {
            self.save(false, actions);
        }
        if save_copy {
            self.save(true, actions);
        }
        if copy {
            self.copy(actions);
        }
        if help {
            self.show_keys = !self.show_keys;
        }
        if enter && self.crop_draft.is_some() {
            self.apply_crop();
        }
        if width != 0.0 {
            let s = &mut self.settings.style;
            if self.settings.tool == Tool::Text || self.settings.tool == Tool::Counter {
                s.font_size = (s.font_size + width * 2.0).clamp(8.0, 300.0);
            } else {
                s.width = (s.width + width).clamp(1.0, 60.0);
            }
            self.apply_settings_to_selection();
        }
        if let Some(n) = swatch {
            self.settings.style.color = SWATCHES[n];
            self.apply_settings_to_selection();
        }
        if escape {
            if self.gesture.is_some() {
                self.gesture = None;
            } else if self.crop_draft.is_some() && self.settings.tool == Tool::Crop {
                self.crop_draft = None;
                self.settings.tool = Tool::Select;
            } else if self.selected.is_some() {
                self.select(None);
            } else if self.show_keys {
                self.show_keys = false;
            } else {
                self.request_close(actions);
            }
        }
    }

    fn type_text(&mut self, index: usize, change: impl FnOnce(&mut String)) {
        if let Some(Annotation {
            shape: Shape::Text(_, text),
            ..
        }) = self.doc.items.get_mut(index)
        {
            change(text);
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui, palette: &Palette, actions: &mut Vec<Action>) {
        ui.horizontal_centered(|ui| {
            let tool = match self.selected.and_then(|i| self.doc.items.get(i)) {
                Some(item) => tool_of(&item.shape),
                None => self.settings.tool,
            };
            let uses = tool.uses();
            let before = self.settings;

            if uses.color {
                for (n, colour) in SWATCHES.iter().enumerate() {
                    let chosen = self.settings.style.color == *colour;
                    let (rect, response) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
                    let painter = ui.painter();
                    painter.rect_filled(rect.shrink(2.0), 4.0, *colour);
                    let ring = if chosen { palette.accent } else { palette.line };
                    painter.rect_stroke(
                        rect.shrink(1.0),
                        5.0,
                        Stroke::new(if chosen { 2.0 } else { 1.0 }, ring),
                        egui::StrokeKind::Inside,
                    );
                    if response.on_hover_text(format!("{}", n + 1)).clicked() {
                        self.settings.style.color = *colour;
                    }
                }
                egui::color_picker::color_edit_button_srgba(
                    ui,
                    &mut self.settings.style.color,
                    egui::color_picker::Alpha::OnlyBlend,
                )
                .on_hover_text("Any colour");
                ui.separator();
            }
            if uses.width {
                ui.label(RichText::new("Width").color(palette.muted));
                ui.add(
                    egui::DragValue::new(&mut self.settings.style.width)
                        .range(1.0..=60.0)
                        .speed(0.2)
                        .suffix(" px"),
                )
                .on_hover_text("[ and ]");
            }
            if uses.font {
                ui.label(RichText::new("Size").color(palette.muted));
                ui.add(
                    egui::DragValue::new(&mut self.settings.style.font_size)
                        .range(8.0..=300.0)
                        .speed(0.5)
                        .suffix(" px"),
                )
                .on_hover_text("[ and ]");
            }
            if uses.fill {
                let mut filled = self.settings.style.fill.is_some();
                if ui.checkbox(&mut filled, "Fill").changed() {
                    self.settings.style.fill = if filled {
                        let [r, g, b, _] = self.settings.style.color.to_array();
                        let alpha = if tool == Tool::Text { 255 } else { 70 };
                        Some(Color32::from_rgba_unmultiplied(r, g, b, alpha))
                    } else {
                        None
                    };
                }
                if let Some(fill) = &mut self.settings.style.fill {
                    egui::color_picker::color_edit_button_srgba(
                        ui,
                        fill,
                        egui::color_picker::Alpha::OnlyBlend,
                    )
                    .on_hover_text("Fill colour");
                }
            }
            if uses.strength {
                match tool {
                    Tool::Blur => {
                        ui.label(RichText::new("Strength").color(palette.muted));
                        ui.add(egui::Slider::new(
                            &mut self.settings.blur_radius,
                            2.0..=60.0,
                        ));
                    }
                    Tool::Pixelate => {
                        ui.label(RichText::new("Block").color(palette.muted));
                        ui.add(
                            egui::Slider::new(&mut self.settings.pixel_size, 3.0..=60.0)
                                .suffix(" px"),
                        );
                    }
                    Tool::Spotlight => {
                        ui.label(RichText::new("Dim").color(palette.muted));
                        let mut percent = self.settings.spot_dim * 100.0;
                        if ui
                            .add(egui::Slider::new(&mut percent, 10.0..=95.0).suffix(" %"))
                            .changed()
                        {
                            self.settings.spot_dim = percent / 100.0;
                        }
                    }
                    _ => {}
                }
            }
            if uses.round {
                ui.checkbox(&mut self.settings.spot_round, "Round");
            }
            if tool == Tool::Crop || self.settings.tool == Tool::Crop {
                if ui
                    .add_enabled(self.crop_draft.is_some(), egui::Button::new("Apply crop"))
                    .on_hover_text("Enter")
                    .clicked()
                {
                    self.apply_crop();
                }
                if ui
                    .add_enabled(self.doc.crop.is_some(), egui::Button::new("Reset"))
                    .clicked()
                {
                    self.reset_crop();
                }
            }
            if self.settings.style.width != before.style.width
                || self.settings.style.color != before.style.color
                || self.settings.style.fill != before.style.fill
                || self.settings.style.font_size != before.style.font_size
                || self.settings.blur_radius != before.blur_radius
                || self.settings.pixel_size != before.pixel_size
                || self.settings.spot_dim != before.spot_dim
                || self.settings.spot_round != before.spot_round
            {
                self.apply_settings_to_selection();
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .button("Save")
                    .on_hover_text("Ctrl+S, overwrites the file")
                    .clicked()
                {
                    self.save(false, actions);
                }
                if ui
                    .button("Save copy")
                    .on_hover_text("Ctrl+Shift+S, keeps the original")
                    .clicked()
                {
                    self.save(true, actions);
                }
                if ui.button("Copy").on_hover_text("Ctrl+C").clicked() {
                    self.copy(actions);
                }
                ui.separator();
                if ui
                    .add_enabled(self.history.can_redo(), egui::Button::new("Redo"))
                    .on_hover_text("Ctrl+Y")
                    .clicked()
                {
                    self.redo();
                }
                if ui
                    .add_enabled(self.history.can_undo(), egui::Button::new("Undo"))
                    .on_hover_text("Ctrl+Z")
                    .clicked()
                {
                    self.undo();
                }
            });
        });
    }

    fn tool_list(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        ui.add_space(8.0);
        for tool in Tool::ALL {
            if matches!(tool, Tool::Text | Tool::Spotlight | Tool::Crop) {
                ui.add_space(6.0);
            }
            let chosen = self.settings.tool == tool;
            let width = ui.available_width();
            let (rect, response) = ui.allocate_exact_size(vec2(width, 26.0), Sense::click());
            let painter = ui.painter();
            if chosen {
                painter.rect_filled(rect, 5.0, palette.accent.gamma_multiply(0.22));
            } else if response.hovered() {
                painter.rect_filled(rect, 5.0, palette.raised);
            }
            painter.text(
                pos2(rect.min.x + 10.0, rect.center().y),
                egui::Align2::LEFT_CENTER,
                tool.label(),
                FontId::proportional(13.0),
                palette.text,
            );
            painter.text(
                pos2(rect.max.x - 10.0, rect.center().y),
                egui::Align2::RIGHT_CENTER,
                format!("{:?}", tool.key()),
                FontId::monospace(11.0),
                palette.muted,
            );
            if response.on_hover_text(tool.hint()).clicked() {
                self.set_tool(tool);
            }
        }
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
            ui.add_space(8.0);
            if ui.button("Shortcuts  F1").clicked() {
                self.show_keys = !self.show_keys;
            }
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        let recent = self
            .message
            .as_ref()
            .filter(|(_, _, at)| at.elapsed().as_secs_f32() < 4.0)
            .map(|(text, error, _)| (text.clone(), *error));
        if recent.is_some() {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(500));
        }
        ui.horizontal_centered(|ui| {
            match recent {
                Some((text, error)) => ui.label(RichText::new(text).size(12.0).color(if error {
                    palette.danger
                } else {
                    palette.accent
                })),
                None => ui.label(
                    RichText::new(self.settings.tool.hint())
                        .size(12.0)
                        .color(palette.muted),
                ),
            };
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let size = self.doc.crop.unwrap_or(self.full()).size();
                ui.label(
                    RichText::new(format!("{} × {}", size.x, size.y))
                        .size(12.0)
                        .color(palette.muted),
                );
            });
        });
    }

    // ------------------------------------------------------------- canvas --

    fn mapping(&self, area: Rect, ppp: f32) -> Mapping {
        let size = self.region.size();
        let scale = self.view.scale(area, size, ppp);
        let origin =
            self.view.screen_pos(area, size, ppp, Pos2::ZERO) - self.region.min.to_vec2() * scale;
        Mapping { origin, scale }
    }

    fn canvas(&mut self, ui: &mut egui::Ui, palette: &Palette) {
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        let (area, response) = ui.allocate_exact_size(ui.available_size(), Sense::click_and_drag());

        // Cropping shows the whole image, everything else the kept part.
        let region = if self.settings.tool == Tool::Crop {
            self.full()
        } else {
            self.doc.crop.unwrap_or(self.full())
        };
        if region != self.region {
            self.region = region;
            self.view.fit();
        }
        let space = ctx.input(|i| i.key_down(Key::Space)) && self.editing.is_none();
        crate::ui::viewer::navigate(
            ui,
            &response,
            &mut self.view,
            area,
            region.size(),
            ppp,
            false,
        );
        if space && response.dragged_by(egui::PointerButton::Primary) {
            self.view
                .pan(area, region.size(), ppp, response.drag_delta());
        }
        let map = self.mapping(area, ppp);

        if space {
            self.gesture = Some(Gesture::Pan);
        } else if matches!(self.gesture, Some(Gesture::Pan)) {
            self.gesture = None;
        }
        self.pointer(&ctx, &response, &map);

        // ---- paint
        let painter = ui.painter_at(area);
        let shown = map.rect(region).intersect(area);
        let painter_img = painter.with_clip_rect(shown);
        crate::ui::canvas::checkerboard(
            &painter_img,
            map.rect(self.full()),
            palette.background.r() < 128,
        );
        painter_img.image(
            self.texture.id(),
            map.rect(self.full()),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );

        let mut used_patches = Vec::new();
        let mut spots = Vec::new();
        let mut dim: f32 = 0.0;
        self.text_sizes.clear();
        for index in 0..self.doc.items.len() {
            let item = self.doc.items[index].clone();
            match &item.shape {
                Shape::Blur(r) | Shape::Pixelate(r) => {
                    let key = hash_of((
                        matches!(item.shape, Shape::Blur(_)),
                        rect_bits(*r),
                        item.style.strength.to_bits(),
                    ));
                    used_patches.push(key);
                    if !self.patches.contains_key(&key) {
                        let made = if matches!(item.shape, Shape::Blur(_)) {
                            effects::blur(&self.base, *r, item.style.strength)
                        } else {
                            effects::pixelate(&self.base, *r, item.style.strength)
                        };
                        if let Some((x, y, patch)) = made {
                            let texture = ctx.load_texture(
                                format!("visura-patch-{key}"),
                                egui::ColorImage::from_rgba_unmultiplied(
                                    [patch.width as usize, patch.height as usize],
                                    &patch.rgba,
                                ),
                                TextureOptions::NEAREST,
                            );
                            let at = Rect::from_min_size(
                                pos2(x as f32, y as f32),
                                vec2(patch.width as f32, patch.height as f32),
                            );
                            self.patches.insert(key, (texture, at));
                        }
                    }
                    if let Some((texture, at)) = self.patches.get(&key) {
                        painter_img.image(
                            texture.id(),
                            map.rect(*at),
                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                            Color32::WHITE,
                        );
                    }
                }
                Shape::Spotlight(r) => {
                    spots.push((*r, item.style.round));
                    dim = dim.max(item.style.strength);
                }
                _ => {
                    let (paints, text_size) = render::shapes(
                        &item,
                        &map,
                        &mut |text: String, size: f32, color: Color32| {
                            painter.layout_no_wrap(text, FontId::proportional(size), color)
                        },
                    );
                    if let Some(size) = text_size {
                        self.text_sizes.insert(index, size);
                    }
                    painter_img.extend(paints);
                }
            }
        }
        self.patches.retain(|key, _| used_patches.contains(key));
        self.paint_spotlight(&ctx, &painter_img, &map, &spots, dim);

        self.paint_selection(&painter, &map, palette);
        self.paint_caret(&painter, &map, &ctx);
        if self.settings.tool == Tool::Crop {
            self.paint_crop(&painter, &map, palette);
        }

        // ---- cursor
        if response.hovered() {
            let icon = if space || matches!(self.gesture, Some(Gesture::Pan)) {
                CursorIcon::Grab
            } else {
                match self.settings.tool {
                    Tool::Select => {
                        let over = response
                            .hover_pos()
                            .map(|p| self.item_at(map.image_pos(p), 6.0 / map.scale).is_some())
                            .unwrap_or(false);
                        if over {
                            CursorIcon::Move
                        } else {
                            CursorIcon::Default
                        }
                    }
                    Tool::Text => CursorIcon::Text,
                    _ => CursorIcon::Crosshair,
                }
            };
            ctx.set_cursor_icon(icon);
        }
        if self.editing.is_some() || self.gesture.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    fn paint_spotlight(
        &mut self,
        ctx: &egui::Context,
        painter: &egui::Painter,
        map: &Mapping,
        spots: &[(Rect, bool)],
        dim: f32,
    ) {
        if spots.is_empty() {
            self.spotlight = None;
            return;
        }
        let key = hash_of((
            spots
                .iter()
                .map(|(r, round)| (rect_bits(*r), *round))
                .collect::<Vec<_>>(),
            dim.to_bits(),
        ));
        if self.spotlight.as_ref().map(|(k, _)| *k) != Some(key) {
            // The preview does not need every pixel; the edge stays soft
            // enough at this size and the mask is rebuilt while dragging.
            let longest = self.base.width.max(self.base.height) as f32;
            let scale = (1400.0 / longest).min(1.0);
            let overlay =
                effects::spotlight_overlay(self.base.width, self.base.height, spots, dim, scale);
            let texture = ctx.load_texture("visura-spotlight", overlay, TextureOptions::LINEAR);
            self.spotlight = Some((key, texture));
        }
        if let Some((_, texture)) = &self.spotlight {
            painter.image(
                texture.id(),
                map.rect(self.full()),
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
    }

    fn paint_selection(&self, painter: &egui::Painter, map: &Mapping, palette: &Palette) {
        let Some(item) = self.selected.and_then(|i| self.doc.items.get(i)) else {
            return;
        };
        let text = self.selected.and_then(|i| self.text_sizes.get(&i).copied());
        let bounds = map.rect(item.bounds(text)).expand(3.0);
        let stroke = Stroke::new(1.0, palette.accent);
        painter.extend(egui::Shape::dashed_line(
            &[
                bounds.left_top(),
                bounds.right_top(),
                bounds.right_bottom(),
                bounds.left_bottom(),
                bounds.left_top(),
            ],
            stroke,
            4.0,
            3.0,
        ));
        for (_, p) in self.handles(item) {
            let r = Rect::from_center_size(map.pos(p), Vec2::splat(8.0));
            painter.rect_filled(r, 1.0, Color32::WHITE);
            painter.rect_stroke(r, 1.0, stroke, egui::StrokeKind::Inside);
        }
    }

    fn paint_caret(&self, painter: &egui::Painter, map: &Mapping, ctx: &egui::Context) {
        let Some(index) = self.editing else {
            return;
        };
        let Some(Annotation {
            shape: Shape::Text(at, text),
            style,
        }) = self.doc.items.get(index)
        else {
            return;
        };
        // Blink, as every text field does.
        if (ctx.input(|i| i.time) * 2.0) as i64 % 2 == 1 {
            return;
        }
        let size = style.font_size * map.scale;
        let lines: Vec<&str> = text.split('\n').collect();
        let last = lines.last().copied().unwrap_or("");
        let row = painter
            .layout_no_wrap("Ag".into(), FontId::proportional(size), style.color)
            .size()
            .y;
        let width = if last.is_empty() {
            0.0
        } else {
            painter
                .layout_no_wrap(last.to_string(), FontId::proportional(size), style.color)
                .size()
                .x
        };
        let top = map.pos(*at) + vec2(width + 1.0, row * (lines.len() - 1) as f32);
        painter.line_segment([top, top + vec2(0.0, row)], Stroke::new(1.5, style.color));
    }

    fn paint_crop(&self, painter: &egui::Painter, map: &Mapping, palette: &Palette) {
        let full = map.rect(self.full());
        let Some(crop) = self.crop_draft else {
            return;
        };
        let c = map.rect(crop);
        let shade = Color32::from_black_alpha(150);
        painter.rect_filled(
            Rect::from_min_max(full.min, pos2(full.max.x, c.min.y)),
            0.0,
            shade,
        );
        painter.rect_filled(
            Rect::from_min_max(pos2(full.min.x, c.max.y), full.max),
            0.0,
            shade,
        );
        painter.rect_filled(
            Rect::from_min_max(pos2(full.min.x, c.min.y), pos2(c.min.x, c.max.y)),
            0.0,
            shade,
        );
        painter.rect_filled(
            Rect::from_min_max(pos2(c.max.x, c.min.y), pos2(full.max.x, c.max.y)),
            0.0,
            shade,
        );
        painter.rect_stroke(
            c,
            0.0,
            Stroke::new(1.5, palette.accent),
            egui::StrokeKind::Outside,
        );
        let label = format!("{:.0} × {:.0}", crop.width(), crop.height());
        painter.text(
            c.left_top() + vec2(0.0, -6.0),
            egui::Align2::LEFT_BOTTOM,
            label,
            FontId::proportional(12.0),
            Color32::WHITE,
        );
    }

    fn handles(&self, item: &Annotation) -> Vec<(Handle, Pos2)> {
        if let Some(r) = item.rect() {
            vec![
                (Handle::Corner(0), r.left_top()),
                (Handle::Corner(1), r.right_top()),
                (Handle::Corner(2), r.right_bottom()),
                (Handle::Corner(3), r.left_bottom()),
            ]
        } else if let Some((a, b)) = item.endpoints() {
            vec![(Handle::Start, a), (Handle::End, b)]
        } else {
            Vec::new()
        }
    }

    /// The topmost annotation at an image point.
    fn item_at(&self, p: Pos2, tolerance: f32) -> Option<usize> {
        (0..self.doc.items.len())
            .rev()
            .find(|&i| self.doc.items[i].hit(p, tolerance, self.text_sizes.get(&i).copied()))
    }

    // ------------------------------------------------------------ pointer --

    fn pointer(&mut self, ctx: &egui::Context, response: &egui::Response, map: &Mapping) {
        let (pressed, down, released, pos, shift, double) = ctx.input(|i| {
            (
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.primary_released(),
                i.pointer.interact_pos(),
                i.modifiers.shift,
                i.pointer
                    .button_double_clicked(egui::PointerButton::Primary),
            )
        });
        let Some(screen) = pos else {
            return;
        };
        let p = map.image_pos(screen);
        let tolerance = 6.0 / map.scale;

        if pressed && response.hovered() && !matches!(self.gesture, Some(Gesture::Pan)) {
            self.press(p, tolerance, map, double);
        }
        if down {
            self.drag(p, shift);
        }
        if released {
            self.release();
        }
    }

    fn press(&mut self, p: Pos2, tolerance: f32, map: &Mapping, double: bool) {
        let tool = self.settings.tool;
        // A click anywhere ends typing; a click on the same text keeps it.
        if let Some(index) = self.editing
            && !(tool == Tool::Text && self.item_at(p, tolerance) == Some(index))
        {
            self.finish_text();
        }
        match tool {
            Tool::Select => {
                // Handles of the selection first, then whatever is on top.
                if let Some(index) = self.selected
                    && let Some(item) = self.doc.items.get(index)
                {
                    let grab = 6.0 / map.scale;
                    if let Some((handle, _)) = self
                        .handles(item)
                        .into_iter()
                        .find(|(_, h)| (*h - p).length() <= grab)
                    {
                        self.gesture = Some(Gesture::Resize {
                            index,
                            handle,
                            before: self.doc.clone(),
                        });
                        return;
                    }
                }
                match self.item_at(p, tolerance) {
                    Some(index) => {
                        self.select(Some(index));
                        if double && matches!(self.doc.items[index].shape, Shape::Text(..)) {
                            self.editing = Some(index);
                            return;
                        }
                        self.gesture = Some(Gesture::Move {
                            index,
                            last: p,
                            before: self.doc.clone(),
                            moved: false,
                        });
                    }
                    None => self.select(None),
                }
            }
            Tool::Text => {
                if let Some(index) = self.item_at(p, tolerance)
                    && matches!(self.doc.items[index].shape, Shape::Text(..))
                {
                    self.editing = Some(index);
                    self.select(Some(index));
                    return;
                }
                self.history.record(&self.doc);
                self.doc.items.push(Annotation {
                    shape: Shape::Text(
                        p - vec2(0.0, self.settings.style.font_size / 2.0),
                        String::new(),
                    ),
                    style: self.style_for(Tool::Text),
                });
                let index = self.doc.items.len() - 1;
                self.editing = Some(index);
                self.selected = Some(index);
                self.restyling = Some(index);
            }
            Tool::Counter => {
                self.history.record(&self.doc);
                let number = self.doc.next_counter();
                self.doc.items.push(Annotation {
                    shape: Shape::Counter(p, number),
                    style: self.style_for(Tool::Counter),
                });
            }
            Tool::Crop => {
                self.gesture = Some(Gesture::Crop { start: p });
                self.crop_draft = None;
            }
            _ => {
                let shape = match tool {
                    Tool::Rectangle => Shape::Rectangle(Rect::from_two_pos(p, p)),
                    Tool::Ellipse => Shape::Ellipse(Rect::from_two_pos(p, p)),
                    Tool::Arrow => Shape::Arrow(p, p),
                    Tool::Line => Shape::Line(p, p),
                    Tool::Pen => Shape::Pen(vec![p]),
                    Tool::Highlighter => Shape::Highlighter(vec![p]),
                    Tool::Spotlight => Shape::Spotlight(Rect::from_two_pos(p, p)),
                    Tool::Blur => Shape::Blur(Rect::from_two_pos(p, p)),
                    Tool::Pixelate => Shape::Pixelate(Rect::from_two_pos(p, p)),
                    Tool::Redact => Shape::Redact(Rect::from_two_pos(p, p)),
                    _ => return,
                };
                let before = self.doc.clone();
                self.doc.items.push(Annotation {
                    shape,
                    style: self.style_for(tool),
                });
                self.gesture = Some(Gesture::Create { start: p, before });
            }
        }
    }

    fn drag(&mut self, p: Pos2, shift: bool) {
        let full = self.full();
        let Some(gesture) = self.gesture.as_mut() else {
            return;
        };
        match gesture {
            Gesture::Create { start, .. } => {
                let start = *start;
                let Some(item) = self.doc.items.last_mut() else {
                    return;
                };
                match &mut item.shape {
                    Shape::Rectangle(r)
                    | Shape::Ellipse(r)
                    | Shape::Spotlight(r)
                    | Shape::Blur(r)
                    | Shape::Pixelate(r)
                    | Shape::Redact(r) => *r = model::drag_rect(start, p, shift),
                    Shape::Arrow(_, b) | Shape::Line(_, b) => {
                        *b = model::snap_line(start, p, shift);
                    }
                    Shape::Pen(points) | Shape::Highlighter(points)
                        if points.last().is_none_or(|last| (*last - p).length() >= 1.0) =>
                    {
                        points.push(p);
                    }
                    _ => {}
                }
            }
            Gesture::Move {
                index,
                last,
                moved,
                before,
            } => {
                let d = p - *last;
                if d != Vec2::ZERO {
                    if !*moved {
                        self.history.record(before);
                        *moved = true;
                    }
                    *last = p;
                    if let Some(item) = self.doc.items.get_mut(*index) {
                        item.translate(d);
                    }
                }
            }
            Gesture::Resize {
                index,
                handle,
                before,
            } => {
                if self.doc == *before {
                    self.history.record(before);
                }
                let Some(item) = self.doc.items.get_mut(*index) else {
                    return;
                };
                match *handle {
                    Handle::Corner(corner) => {
                        if let Some(r) = item.rect() {
                            let opposite = [
                                r.right_bottom(),
                                r.left_bottom(),
                                r.left_top(),
                                r.right_top(),
                            ][corner];
                            let new = model::drag_rect(opposite, p, shift);
                            item.set_rect(new);
                            // Dragging past the opposite corner turns the
                            // rectangle over; keep holding the same point.
                            let corners = [
                                new.left_top(),
                                new.right_top(),
                                new.right_bottom(),
                                new.left_bottom(),
                            ];
                            if let Some(c) = corners.iter().position(|q| *q == opposite) {
                                *handle = Handle::Corner((c + 2) % 4);
                            }
                        }
                    }
                    Handle::Start => {
                        if let Some((_, b)) = item.endpoints() {
                            item.set_endpoints(model::snap_line(b, p, shift), b);
                        }
                    }
                    Handle::End => {
                        if let Some((a, _)) = item.endpoints() {
                            item.set_endpoints(a, model::snap_line(a, p, shift));
                        }
                    }
                }
            }
            Gesture::Crop { start } => {
                self.crop_draft = Some(Rect::from_two_pos(*start, p).intersect(full));
            }
            Gesture::Pan => {}
        }
    }

    fn release(&mut self) {
        match self.gesture.take() {
            Some(Gesture::Create { before, .. }) => {
                let keep = match self.doc.items.last() {
                    Some(item) => match &item.shape {
                        Shape::Pen(_) | Shape::Highlighter(_) => true,
                        Shape::Arrow(a, b) | Shape::Line(a, b) => (*b - *a).length() >= 3.0,
                        _ => item
                            .rect()
                            .is_some_and(|r| r.width() >= 3.0 && r.height() >= 3.0),
                    },
                    None => false,
                };
                if keep {
                    self.history.record(&before);
                } else {
                    self.doc.items.pop();
                }
            }
            Some(Gesture::Crop { .. }) => {
                if self
                    .crop_draft
                    .is_some_and(|r| r.width() < 3.0 || r.height() < 3.0)
                {
                    self.crop_draft = None;
                }
            }
            Some(Gesture::Pan) => self.gesture = Some(Gesture::Pan),
            _ => {}
        }
    }

    // ------------------------------------------------------------ dialogs --

    fn close_dialog(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        if !self.confirm_close {
            return;
        }
        let mut choice = None;
        egui::Modal::new(egui::Id::new("visura-editor-close")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.heading("Save the changes?");
            ui.add_space(6.0);
            ui.label("Closing without saving loses this edit.");
            ui.add_space(14.0);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Save").clicked() {
                    choice = Some(0);
                }
                if ui.button("Don't save").clicked() {
                    choice = Some(1);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(2);
                }
            });
        });
        if ctx.input(|i| i.key_pressed(Key::Escape)) {
            choice = Some(2);
        }
        match choice {
            Some(0) => {
                self.confirm_close = false;
                if self.save(false, actions) {
                    actions.push(Action::Close);
                }
            }
            Some(1) => {
                self.confirm_close = false;
                actions.push(Action::Close);
            }
            Some(_) => self.confirm_close = false,
            None => {}
        }
    }

    fn keys_window(&mut self, ctx: &egui::Context) {
        if !self.show_keys {
            return;
        }
        let mut open = true;
        egui::Window::new("Shortcuts")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::RIGHT_BOTTOM, vec2(-16.0, -40.0))
            .show(ctx, |ui| {
                let room = ctx.content_rect().height() - 180.0;
                egui::ScrollArea::vertical()
                    .max_height(room.max(120.0))
                    .show(ui, |ui| {
                        egui::Grid::new("visura-editor-keys")
                            .num_columns(2)
                            .spacing([14.0, 1.0])
                            .show(ui, |ui| {
                                for tool in Tool::ALL {
                                    ui.monospace(format!("{:?}", tool.key()));
                                    ui.label(tool.label());
                                    ui.end_row();
                                }
                                for (keys, what) in [
                                    ("1 – 8", "Colour"),
                                    ("[  ]", "Thinner / thicker, smaller / larger text"),
                                    ("Shift", "Square, circle, 45° lines"),
                                    ("Space + drag", "Move the view (also middle mouse)"),
                                    ("Wheel", "Zoom"),
                                    ("Ctrl+0 / Ctrl+1", "Fit / actual size"),
                                    ("Del", "Remove the selection"),
                                    ("Ctrl+Z / Ctrl+Y", "Undo / redo"),
                                    ("Enter", "Apply the crop"),
                                    ("Ctrl+Enter / Esc", "Finish text"),
                                    ("Ctrl+C", "Copy the result"),
                                    ("Ctrl+S", "Save over the file"),
                                    ("Ctrl+Shift+S", "Save a copy"),
                                    ("Esc", "Cancel, deselect, close"),
                                ] {
                                    ui.monospace(keys);
                                    ui.label(what);
                                    ui.end_row();
                                }
                            });
                    });
            });
        if !open {
            self.show_keys = false;
        }
    }
}

impl Mapping {
    pub fn image_pos(&self, p: Pos2) -> Pos2 {
        ((p - self.origin) / self.scale).to_pos2()
    }
}
