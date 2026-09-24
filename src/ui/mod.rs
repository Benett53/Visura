//! The window: sidebar, pages, notices and the dialogs.

pub mod canvas;
pub mod history;
pub mod settings;
pub mod theme;
pub mod viewer;

use egui::{Align, Align2, CornerRadius, FontId, Id, Layout, RichText, Sense, vec2};

use crate::app::{App, OpenRequest, Page};
use crate::capture::Mode;
use crate::hotkeys;

const SIDEBAR_WIDTH: f32 = 178.0;

pub fn shell(app: &mut App, ui: &mut egui::Ui) {
    let full = ui.max_rect();
    sidebar(app, ui);

    match app.page {
        Page::Recent => {
            selection_bar(app, ui, Id::new("visura-selection"));
            egui::CentralPanel::default().show(ui, |ui| app.recent_page(ui));
        }
        Page::Settings => {
            egui::CentralPanel::default().show(ui, |ui| app.settings_page(ui));
        }
    }

    toast(app, ui, full);
    // The dialogs follow the window that is in charge, so a delete started in
    // the history does not ask for confirmation behind it.
    if !app.history_open {
        delete_dialog(app, &ui.ctx().clone());
        rename_dialog(app, &ui.ctx().clone());
    }
    history_window(app, ui);
    open_requested(app, ui.ctx());
    viewer_window(app, ui);
    editor_window(app, ui);
}

// -------------------------------------------------------- viewer and editor --

const VIEWER: &str = "visura-viewer";
const EDITOR: &str = "visura-editor";

fn open_requested(app: &mut App, ctx: &egui::Context) {
    let Some(request) = app.open_request.take() else {
        return;
    };
    match request {
        OpenRequest::View(path) => {
            app.viewer = Some(viewer::Viewer::open(ctx, path));
            ctx.send_viewport_cmd_to(
                egui::ViewportId::from_hash_of(VIEWER),
                egui::ViewportCommand::Focus,
            );
        }
        OpenRequest::Edit(path) => {
            let editor_id = egui::ViewportId::from_hash_of(EDITOR);
            if let Some(open) = &app.editor {
                if open.path == path {
                    ctx.send_viewport_cmd_to(editor_id, egui::ViewportCommand::Focus);
                    return;
                }
                if open.is_dirty() {
                    app.warn("Save or close the edit that is open first");
                    ctx.send_viewport_cmd_to(editor_id, egui::ViewportCommand::Focus);
                    return;
                }
            }
            match crate::editor::Editor::open(
                ctx,
                path,
                app.editor_settings,
                app.config.jpeg_quality,
            ) {
                Ok(editor) => {
                    app.editor = Some(editor);
                    ctx.send_viewport_cmd_to(editor_id, egui::ViewportCommand::Focus);
                }
                Err(e) => app.warn(e),
            }
        }
    }
}

fn viewer_window(app: &mut App, ui: &mut egui::Ui) {
    let Some(open) = &app.viewer else {
        return;
    };
    let path = open.path.clone();
    let ctx = ui.ctx().clone();
    let builder = egui::ViewportBuilder::default()
        .with_title(open.title())
        .with_inner_size([1100.0, 760.0])
        .with_min_inner_size([420.0, 300.0]);
    let position = app.position_of(&path);
    let palette = app.palette();
    let mut actions = Vec::new();

    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(VIEWER), builder, |ui, _| {
        if ui.ctx().input(|i| i.viewport().close_requested()) {
            actions.push(viewer::Action::Close);
            return;
        }
        if let Some(viewer) = &mut app.viewer {
            actions = viewer.ui(ui, &palette, position);
        }
    });

    for action in actions {
        match action {
            viewer::Action::Close => app.viewer = None,
            viewer::Action::Edit(path) => {
                app.viewer = None;
                app.open_request = Some(OpenRequest::Edit(path));
            }
            viewer::Action::Step(step) => {
                if let Some(next) = app.neighbour_of(&path, step) {
                    app.selection.clear();
                    app.selection.insert(next.clone());
                    app.viewer = Some(viewer::Viewer::open(&ctx, next));
                }
            }
            viewer::Action::Copy(path) => app.copy_image_of(&path),
            viewer::Action::OpenExternally(path) => crate::platform::open_path(&path),
        }
    }
}

fn editor_window(app: &mut App, ui: &mut egui::Ui) {
    let Some(open) = &app.editor else {
        return;
    };
    let ctx = ui.ctx().clone();
    let builder = egui::ViewportBuilder::default()
        .with_title(open.title())
        .with_inner_size([1280.0, 820.0])
        .with_min_inner_size([760.0, 480.0]);
    let palette = app.palette();
    let mut actions = Vec::new();

    ctx.show_viewport_immediate(egui::ViewportId::from_hash_of(EDITOR), builder, |ui, _| {
        let inner = ui.ctx().clone();
        let Some(editor) = &mut app.editor else {
            return;
        };
        if inner.input(|i| i.viewport().close_requested()) {
            if editor.is_dirty() {
                inner.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
            editor.request_close(&mut actions);
        }
        actions.extend(editor.ui(ui, &palette));
    });

    for action in actions {
        match action {
            crate::editor::Action::Close => {
                if let Some(editor) = app.editor.take() {
                    app.editor_settings = editor.settings;
                }
            }
            crate::editor::Action::Saved(path) => {
                app.thumbs.forget(&path);
                app.refresh();
                app.selection.clear();
                app.selection.insert(path);
            }
            crate::editor::Action::Note(text) => {
                if let Some(editor) = &mut app.editor {
                    editor.flash(text.clone(), false);
                }
                app.note(text);
            }
            crate::editor::Action::Warn(text) => {
                if let Some(editor) = &mut app.editor {
                    editor.flash(text.clone(), true);
                }
                app.warn(text);
            }
        }
    }
}

// ------------------------------------------------------------------ sidebar --

fn sidebar(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette();
    egui::Panel::left(Id::new("visura-sidebar"))
        .exact_size(SIDEBAR_WIDTH)
        .resizable(false)
        .show(ui, |ui| {
            ui.add_space(10.0);
            ui.label(RichText::new("Visura").size(15.0).strong());
            ui.add_space(12.0);

            for (label, mode) in [
                ("Capture region", Mode::Region),
                ("Capture window", Mode::ActiveWindow),
                ("Capture screen", Mode::Fullscreen),
            ] {
                if nav_item(ui, &palette, label, false).clicked() {
                    app.request_capture(mode);
                }
            }

            ui.add_space(10.0);
            ui.separator();
            ui.add_space(6.0);

            if nav_item(ui, &palette, "All screenshots", app.history_open).clicked() {
                app.history_open = !app.history_open;
                if app.history_open {
                    app.refresh();
                }
            }
            if nav_item(ui, &palette, "Open folder", false).clicked() {
                let folder = app.config.folder.clone();
                let _ = std::fs::create_dir_all(&folder);
                crate::platform::open_path(&folder);
            }

            ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                ui.add_space(10.0);
                let on_settings = app.page == Page::Settings;
                if nav_item(ui, &palette, "Settings", on_settings).clicked() {
                    if on_settings {
                        app.page = Page::Recent;
                    } else {
                        app.draft = app.config.clone();
                        app.recording = None;
                        app.page = Page::Settings;
                    }
                }
            });
        });
}

/// One row of the sidebar.
///
/// The shortcut used to be printed on the right. It did not survive a
/// combination of more than one key: the text ran into the label and was cut
/// off. The settings screen is where shortcuts belong.
fn nav_item(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    label: &str,
    selected: bool,
) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 30.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if selected {
            painter.rect_filled(
                rect,
                CornerRadius::same(5),
                palette.accent.gamma_multiply(0.22),
            );
        } else if response.hovered() {
            painter.rect_filled(rect, CornerRadius::same(5), palette.raised);
        }
        painter.text(
            egui::pos2(rect.min.x + 10.0, rect.center().y),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(13.0),
            palette.text,
        );
    }
    response
}

// ---------------------------------------------------------- history window --

fn history_window(app: &mut App, ui: &mut egui::Ui) {
    if !app.history_open {
        return;
    }
    let ctx = ui.ctx().clone();
    let builder = egui::ViewportBuilder::default()
        .with_title("Visura - all screenshots")
        .with_inner_size([940.0, 640.0])
        .with_min_inner_size([460.0, 320.0]);

    ctx.show_viewport_immediate(
        egui::ViewportId::from_hash_of("visura-history"),
        builder,
        |ui, _class| {
            let inner = ui.ctx().clone();
            if inner.input(|i| i.viewport().close_requested()) {
                app.history_open = false;
                return;
            }
            selection_bar(app, ui, Id::new("visura-history-selection"));
            egui::CentralPanel::default().show(ui, |ui| app.history_page(ui));
            delete_dialog(app, &inner);
            rename_dialog(app, &inner);
        },
    );
}

// ----------------------------------------------------------- selection bar --

fn selection_bar(app: &mut App, ui: &mut egui::Ui, id: Id) {
    let count = app.selection.len();
    if count == 0 {
        return;
    }
    let palette = app.palette();

    egui::Panel::bottom(id).exact_size(44.0).show(ui, |ui| {
        ui.horizontal_centered(|ui| {
            ui.label(
                RichText::new(if count == 1 {
                    "1 selected".to_string()
                } else {
                    format!("{count} selected")
                })
                .color(palette.muted),
            );
            ui.separator();

            if count == 1 {
                let path = app.selected_paths().remove(0);
                if ui.button("Open").clicked() {
                    app.open_request = Some(crate::app::OpenRequest::View(path.clone()));
                }
                if ui.button("Edit").clicked() {
                    app.open_request = Some(crate::app::OpenRequest::Edit(path.clone()));
                }
                if ui.button("Copy image").clicked() {
                    app.copy_image_of(&path);
                }
                if ui.button("Rename").clicked() {
                    let stem = path
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    app.renaming = Some((path, stem));
                }
            }
            if ui.button("Show in folder").clicked()
                && let Some(path) = app.selected_paths().first()
            {
                crate::platform::reveal_in_file_manager(path);
            }
            if ui.button("Copy path").clicked() {
                let joined = app
                    .selected_paths()
                    .iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("\n");
                match crate::clipboard::copy_text(&joined) {
                    Ok(()) => app.note("Path copied"),
                    Err(e) => app.warn(e),
                }
            }

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .button(RichText::new("Delete").color(palette.danger))
                    .on_hover_text("Moves to the recycle bin")
                    .clicked()
                {
                    app.delete_selected();
                }
                if ui.button("Clear selection").clicked() {
                    app.selection.clear();
                }
            });
        });
    });
}

// ------------------------------------------------------------------- toast --

fn toast(app: &mut App, ui: &mut egui::Ui, full: egui::Rect) {
    let Some(toast) = &app.toast else {
        return;
    };
    let palette = app.palette();
    let colour = if toast.error {
        palette.danger
    } else {
        palette.accent
    };
    let text = toast.text.clone();
    let ctx = ui.ctx().clone();

    egui::Area::new(Id::new("visura-toast"))
        .fixed_pos(egui::pos2(full.center().x, full.max.y - 20.0))
        .pivot(Align2::CENTER_BOTTOM)
        .interactable(false)
        .order(egui::Order::Foreground)
        .show(&ctx, |ui| {
            egui::Frame::popup(ui.style())
                .fill(palette.raised)
                .stroke(egui::Stroke::new(1.0, colour.gamma_multiply(0.6)))
                .corner_radius(CornerRadius::same(7))
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(RichText::new(text).color(palette.text))
                            .wrap_mode(egui::TextWrapMode::Extend),
                    );
                });
        });
    // Let the notice disappear on its own rather than on the next mouse move.
    ctx.request_repaint_after(std::time::Duration::from_millis(400));
}

// ----------------------------------------------------------------- dialogs --

fn delete_dialog(app: &mut App, ctx: &egui::Context) {
    if app.pending_delete.is_empty() {
        return;
    }
    let count = app.pending_delete.len();
    let mut confirmed = false;
    let mut cancelled = false;

    egui::Modal::new(Id::new("visura-delete")).show(ctx, |ui| {
        ui.set_width(360.0);
        ui.heading(if count == 1 {
            "Delete this screenshot?".to_string()
        } else {
            format!("Delete {count} screenshots?")
        });
        ui.add_space(6.0);
        ui.label("They go to the recycle bin and can be put back from there.");
        ui.add_space(14.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("Delete").clicked() {
                confirmed = true;
            }
            if ui.button("Cancel").clicked() {
                cancelled = true;
            }
        });
    });

    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        cancelled = true;
    }
    if confirmed {
        let paths = std::mem::take(&mut app.pending_delete);
        app.perform_delete(paths);
    } else if cancelled {
        app.pending_delete.clear();
    }
}

fn rename_dialog(app: &mut App, ctx: &egui::Context) {
    let Some((_, current)) = &app.renaming else {
        return;
    };
    let mut name = current.clone();
    let mut confirmed = false;
    let mut cancelled = false;

    egui::Modal::new(Id::new("visura-rename")).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.heading("Rename");
        ui.add_space(8.0);
        let response = ui.add(
            egui::TextEdit::singleline(&mut name)
                .desired_width(f32::INFINITY)
                .hint_text("New name"),
        );
        response.request_focus();
        if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            confirmed = true;
        }
        ui.add_space(4.0);
        ui.small("The file extension stays as it is.");
        ui.add_space(12.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("Save").clicked() {
                confirmed = true;
            }
            if ui.button("Cancel").clicked() {
                cancelled = true;
            }
        });
    });

    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        cancelled = true;
    }
    if let Some((_, stored)) = &mut app.renaming {
        *stored = name;
    }
    if confirmed {
        app.finish_rename();
    } else if cancelled {
        app.renaming = None;
    }
}

/// Shared by the settings screen: what a shortcut field shows right now.
pub fn shortcut_text(shortcut: &str, recording: bool) -> String {
    if recording {
        "Press a key…".to_string()
    } else {
        hotkeys::describe(shortcut)
    }
}
