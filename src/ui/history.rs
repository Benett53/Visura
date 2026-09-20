//! The thumbnail grids.
//!
//! The main window shows the last handful of shots and nothing else. The full
//! library, with day headings and a search box, lives in a window of its own,
//! so the thing people open twenty times a day stays small.
//!
//! Tiles are laid out by hand rather than with a grid widget so that rows and
//! whole days can be skipped when they are scrolled out of view. A library
//! with a few thousand shots then costs the same per frame as an empty one,
//! and no thumbnail is decoded for something nobody is looking at.

use egui::{Color32, CornerRadius, FontId, Rect, RichText, Sense, Stroke, Vec2, vec2};

use crate::app::App;
use crate::ui::theme;

const GAP: f32 = 10.0;
const CAPTION: f32 = 38.0;
const HEADING: f32 = 26.0;

/// Which list a tile belongs to. Shift-clicking only spans one of them.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum List {
    /// The newest shots, shown in the main window.
    Recent,
    /// Everything that matches the search, shown in the history window.
    All,
}

impl App {
    pub(crate) fn palette(&self) -> theme::Palette {
        theme::palette(self.config.ui.theme, self.config.ui.accent)
    }

    fn list_len(&self, list: List) -> usize {
        match list {
            List::Recent => self.shots.len().min(self.config.ui.recent_count),
            List::All => self.view.len(),
        }
    }

    fn index_at(&self, list: List, position: usize) -> Option<usize> {
        match list {
            List::Recent => (position < self.list_len(list)).then_some(position),
            List::All => self.view.get(position).copied(),
        }
    }

    // ------------------------------------------------------ main window ----

    /// The newest shots, no headings, no search.
    pub fn recent_page(&mut self, ui: &mut egui::Ui) {
        self.history_shortcuts(ui, List::Recent);

        if self.shots.is_empty() {
            self.empty_state(ui);
            return;
        }

        let palette = self.palette();
        let shown = self.list_len(List::Recent);
        let total = self.shots.len();

        ui.horizontal(|ui| {
            ui.label(RichText::new("Recent").size(14.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if total > shown {
                    ui.label(
                        RichText::new(format!("{shown} of {total}"))
                            .color(palette.muted)
                            .size(11.5),
                    );
                }
            });
        });
        ui.add_space(4.0);

        let tile_w = self.config.ui.thumb_size;
        let tile_h = tile_w * 0.68 + CAPTION;

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(GAP, GAP);
                let columns = columns_for(ui.available_width(), tile_w);
                for row in 0..shown.div_ceil(columns) {
                    ui.horizontal(|ui| {
                        for position in row * columns..((row + 1) * columns).min(shown) {
                            self.tile(ui, List::Recent, position, vec2(tile_w, tile_h));
                        }
                    });
                }
                ui.add_space(8.0);
            });
    }

    // --------------------------------------------------- history window ----

    /// The whole library, grouped by day, with a search box.
    pub fn history_page(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette();

        ui.horizontal(|ui| {
            // Taken out and put back so the view can be rebuilt while the
            // field is still borrowed by the widget.
            let mut search = std::mem::take(&mut self.search);
            let response = ui.add(
                egui::TextEdit::singleline(&mut search)
                    .hint_text("Search")
                    .desired_width(240.0),
            );
            self.search = search;
            if response.changed() {
                self.rebuild_view();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    RichText::new(format!("{} shots", self.view.len()))
                        .color(palette.muted)
                        .size(11.5),
                );
            });
        });
        ui.add_space(6.0);

        self.history_shortcuts(ui, List::All);

        if self.view.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(64.0);
                ui.label(
                    RichText::new(if self.shots.is_empty() {
                        "No screenshots yet"
                    } else {
                        "Nothing found"
                    })
                    .color(palette.muted)
                    .size(15.0),
                );
            });
            return;
        }

        let tile_w = self.config.ui.thumb_size;
        let tile_h = tile_w * 0.68 + CAPTION;

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing = vec2(GAP, GAP);
                let width = ui.available_width();
                let columns = columns_for(width, tile_w);
                let clip = ui.clip_rect();

                for (label, range) in self.groups.clone() {
                    let rows = range.len().div_ceil(columns);
                    let height = HEADING + rows as f32 * (tile_h + GAP);
                    let top = ui.cursor().top();

                    // Whole day off screen: reserve the space and move on.
                    if top + height < clip.top() - 200.0 || top > clip.bottom() + 200.0 {
                        ui.allocate_exact_size(vec2(width, height), Sense::hover());
                        continue;
                    }

                    ui.label(RichText::new(label).color(palette.muted).size(12.5));

                    for row in 0..rows {
                        let row_top = ui.cursor().top();
                        if row_top + tile_h < clip.top() - 100.0 || row_top > clip.bottom() + 100.0
                        {
                            ui.allocate_exact_size(vec2(width, tile_h), Sense::hover());
                            continue;
                        }
                        ui.horizontal(|ui| {
                            let start = range.start + row * columns;
                            for position in start..(start + columns).min(range.end) {
                                self.tile(ui, List::All, position, vec2(tile_w, tile_h));
                            }
                        });
                    }
                    ui.add_space(6.0);
                }
                ui.add_space(12.0);
            });
    }

    // ---------------------------------------------------------- shared ----

    fn history_shortcuts(&mut self, ui: &mut egui::Ui, list: List) {
        if self.renaming.is_some() || !self.pending_delete.is_empty() {
            return;
        }
        let (delete, escape, select_all) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Delete),
                i.key_pressed(egui::Key::Escape),
                i.modifiers.command && i.key_pressed(egui::Key::A),
            )
        });
        if delete {
            self.delete_selected();
        }
        if escape {
            self.selection.clear();
        }
        if select_all {
            self.selection = (0..self.list_len(list))
                .filter_map(|p| self.index_at(list, p))
                .map(|i| self.shots[i].path.clone())
                .collect();
        }
    }

    fn empty_state(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette();
        let folder = self.config.folder.clone();
        let shortcut = self.config.hotkeys.region.clone();
        ui.vertical_centered(|ui| {
            ui.add_space(72.0);
            ui.label(
                RichText::new("No screenshots yet")
                    .size(17.0)
                    .color(palette.text),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new(if shortcut.is_empty() {
                    "Use Capture region on the left, or set a shortcut in the settings.".to_string()
                } else {
                    format!("{shortcut} captures a region.")
                })
                .color(palette.muted),
            );
            ui.add_space(20.0);
            if ui.button("Open folder").clicked() {
                let _ = std::fs::create_dir_all(&folder);
                crate::platform::open_path(&folder);
            }
        });
    }

    fn tile(&mut self, ui: &mut egui::Ui, list: List, position: usize, size: Vec2) {
        let Some(index) = self.index_at(list, position) else {
            return;
        };
        let (path, name, clock, human_size, modified, bytes) = {
            let shot = &self.shots[index];
            (
                shot.path.clone(),
                shot.name.clone(),
                shot.clock(),
                shot.human_size(),
                shot.modified,
                shot.bytes,
            )
        };
        let palette = self.palette();
        let selected = self.selection.contains(&path);

        let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
        if ui.is_rect_visible(rect) {
            let background = if selected {
                palette.accent.gamma_multiply(0.22)
            } else if response.hovered() {
                palette.raised
            } else {
                palette.panel
            };
            let painter = ui.painter();
            painter.rect_filled(rect, CornerRadius::same(7), background);
            if selected {
                painter.rect_stroke(
                    rect,
                    CornerRadius::same(7),
                    Stroke::new(1.5, palette.accent),
                    egui::StrokeKind::Inside,
                );
            }

            let image_area = Rect::from_min_size(
                rect.min + Vec2::splat(6.0),
                vec2(size.x - 12.0, size.y - CAPTION - 8.0),
            );

            match self.thumbs.get(&path, modified, bytes) {
                Some(texture) => {
                    let texture_size = texture.size_vec2();
                    let scale = (image_area.width() / texture_size.x)
                        .min(image_area.height() / texture_size.y)
                        .min(1.0);
                    let drawn = Rect::from_center_size(image_area.center(), texture_size * scale);
                    ui.painter().image(
                        texture.id(),
                        drawn,
                        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                }
                None => {
                    ui.painter().rect_filled(
                        image_area,
                        CornerRadius::same(4),
                        palette.raised.gamma_multiply(0.6),
                    );
                }
            }

            // Two lines, measured rather than guessed: a file name that does
            // not fit is cut by the text layout, not by an estimate of how
            // wide a character is.
            let text_width = size.x - 16.0;
            let name_galley = truncated(ui, &name, 11.5, palette.text, text_width);
            let meta_galley = truncated(
                ui,
                &format!("{clock}  ·  {human_size}"),
                10.5,
                palette.muted,
                text_width,
            );
            let painter = ui.painter();
            painter.galley(
                egui::pos2(rect.min.x + 8.0, rect.max.y - 34.0),
                name_galley,
                palette.text,
            );
            painter.galley(
                egui::pos2(rect.min.x + 8.0, rect.max.y - 18.0),
                meta_galley,
                palette.muted,
            );
        }

        // ------------------------------------------------------ input ----
        if response.clicked() {
            let modifiers = ui.input(|i| i.modifiers);
            if modifiers.command {
                if !self.selection.remove(&path) {
                    self.selection.insert(path.clone());
                }
            } else if modifiers.shift
                && let Some((anchor_list, anchor)) = self.last_clicked
                && anchor_list == list
            {
                let (from, to) = (anchor.min(position), anchor.max(position));
                for p in from..=to {
                    if let Some(i) = self.index_at(list, p) {
                        self.selection.insert(self.shots[i].path.clone());
                    }
                }
            } else {
                self.selection.clear();
                self.selection.insert(path.clone());
            }
            self.last_clicked = Some((list, position));
        }

        if response.double_clicked() {
            crate::platform::open_path(&path);
        }

        if response.drag_started() {
            // Dragging something that is not selected should drag that one
            // thing, which is what every file manager does.
            if !self.selection.contains(&path) {
                self.selection.clear();
                self.selection.insert(path.clone());
                self.last_clicked = Some((list, position));
            }
            self.start_drag();
        }

        response.context_menu(|ui| {
            if !self.selection.contains(&path) {
                self.selection.clear();
                self.selection.insert(path.clone());
            }
            if ui.button("Open").clicked() {
                crate::platform::open_path(&path);
                ui.close();
            }
            if ui.button("Show in folder").clicked() {
                crate::platform::reveal_in_file_manager(&path);
                ui.close();
            }
            if ui.button("Copy image").clicked() {
                self.copy_image_of(&path);
                ui.close();
            }
            if ui.button("Copy path").clicked() {
                match crate::clipboard::copy_text(&path.to_string_lossy()) {
                    Ok(()) => self.note("Path copied"),
                    Err(e) => self.warn(e),
                }
                ui.close();
            }
            ui.separator();
            if ui.button("Rename").clicked() {
                let stem = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.renaming = Some((path.clone(), stem));
                ui.close();
            }
            if ui
                .button(RichText::new("Delete").color(palette.danger))
                .clicked()
            {
                self.delete_selected();
                ui.close();
            }
        });

        response.on_hover_text(&name);
    }
}

fn columns_for(width: f32, tile_width: f32) -> usize {
    (((width + GAP) / (tile_width + GAP)).floor() as usize).max(1)
}

/// Lay out one line, cut with an ellipsis when it does not fit.
fn truncated(
    ui: &egui::Ui,
    text: &str,
    size: f32,
    colour: Color32,
    width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(
        text.to_owned(),
        egui::TextFormat::simple(FontId::proportional(size), colour),
    );
    job.wrap.max_width = width.max(1.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    ui.ctx().fonts_mut(|f| f.layout_job(job))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_never_drop_below_one() {
        assert_eq!(columns_for(10.0, 168.0), 1);
        assert_eq!(columns_for(0.0, 168.0), 1);
    }

    #[test]
    fn columns_grow_with_the_window() {
        assert_eq!(columns_for(168.0, 168.0), 1);
        assert_eq!(columns_for(346.0, 168.0), 2);
        assert_eq!(columns_for(900.0, 168.0), 5);
    }
}
