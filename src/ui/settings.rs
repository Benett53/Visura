//! The settings screen.
//!
//! Changes are made to a draft and only take effect on save, so a half
//! finished edit cannot break a shortcut or send the next screenshot
//! somewhere unexpected.

use egui::{Align, Layout, RichText};

use crate::app::{App, Page};
use crate::config::{Accent, Format, Theme};
use crate::hotkeys::Action;
use crate::platform::LocalTime;
use crate::ui::theme;
use crate::{hotkeys, naming};

impl App {
    pub fn settings_page(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette();
        let dirty = self.draft != self.config;

        egui::Panel::bottom(egui::Id::new("settings-actions"))
            .exact_size(50.0)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    if ui.button("Open config file").clicked() {
                        let path = crate::config::Config::path();
                        if !path.exists() {
                            let _ = self.config.save();
                        }
                        crate::platform::open_path(&path);
                    }
                    if ui.button("Reset to defaults").clicked() {
                        self.draft = crate::config::Config::default();
                    }

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add_enabled(dirty, egui::Button::new("Save")).clicked() {
                            let ctx = ui.ctx().clone();
                            self.stop_recording();
                            self.apply_settings(&ctx);
                            self.page = Page::Recent;
                        }
                        if ui.button(if dirty { "Discard" } else { "Close" }).clicked() {
                            self.stop_recording();
                            self.draft = self.config.clone();
                            self.page = Page::Recent;
                        }
                        if dirty {
                            ui.label(RichText::new("unsaved").color(palette.muted).size(11.5));
                        }
                    });
                });
            });

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.set_max_width(660.0);
                self.storage_section(ui, &palette);
                self.after_capture_section(ui, &palette);
                self.shortcut_section(ui, &palette);
                self.overlay_section(ui, &palette);
                self.appearance_section(ui, &palette);
                ui.add_space(16.0);
            });
    }

    /// Put the global shortcuts back after recording, bound or not.
    pub fn stop_recording(&mut self) {
        if self.recording.take().is_some() {
            let keys = self.config.hotkeys.clone();
            self.hotkeys.apply(&keys);
        }
    }

    // --------------------------------------------------------- sections ----

    fn storage_section(&mut self, ui: &mut egui::Ui, palette: &theme::Palette) {
        section(ui, palette, "Storage", |ui| {
            ui.horizontal(|ui| {
                let mut folder = self.draft.folder.to_string_lossy().into_owned();
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut folder)
                            .desired_width(390.0)
                            .hint_text("Folder"),
                    )
                    .changed()
                {
                    self.draft.folder = folder.into();
                }
                if ui.button("Choose …").clicked()
                    && let Some(picked) = crate::platform::pick_folder(&self.draft.folder)
                {
                    self.draft.folder = picked;
                }
                if ui.button("Open").clicked() {
                    let _ = std::fs::create_dir_all(&self.draft.folder);
                    crate::platform::open_path(&self.draft.folder);
                }
            });

            ui.add_space(10.0);
            ui.checkbox(
                &mut self.draft.anonymous_names,
                "Anonymous file names (random characters, nothing about the shot)",
            );
            if self.draft.anonymous_names {
                ui.label(
                    RichText::new(
                        "    The folders stay as they are, so shots are still easy to find.",
                    )
                    .color(palette.muted)
                    .size(11.5),
                );
            }

            ui.add_space(6.0);
            let named = !self.draft.anonymous_names;
            labelled(ui, "Subfolder", |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.draft.subfolder)
                        .desired_width(240.0)
                        .hint_text("leave empty for one flat folder"),
                );
            });
            ui.add_enabled_ui(named, |ui| {
                labelled(ui, "File name", |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.draft.filename).desired_width(240.0),
                    );
                });
            });

            ui.add_space(6.0);
            ui.label(
                RichText::new(self.name_preview())
                    .color(palette.muted)
                    .size(11.5)
                    .monospace(),
            );

            if named {
                ui.add_space(6.0);
                ui.collapsing("Placeholders", |ui| {
                    egui::Grid::new("tokens")
                        .num_columns(2)
                        .spacing([16.0, 4.0])
                        .show(ui, |ui| {
                            for (token, description) in naming::TOKENS {
                                ui.label(RichText::new(*token).monospace().color(palette.accent));
                                ui.label(RichText::new(*description).color(palette.muted));
                                ui.end_row();
                            }
                        });
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(
                            "A placeholder that has nothing to fill in leaves no gap behind: \
                             a full screen shot has no program, so %app_%Y-%m-%d becomes \
                             just the date.",
                        )
                        .color(palette.muted)
                        .size(11.5),
                    );
                });
            }

            ui.add_space(10.0);
            labelled(ui, "Format", |ui| {
                ui.horizontal(|ui| {
                    for format in [Format::Png, Format::Jpeg] {
                        ui.selectable_value(&mut self.draft.format, format, format.label());
                    }
                });
            });
            if self.draft.format == Format::Jpeg {
                labelled(ui, "Quality", |ui| {
                    ui.add(egui::Slider::new(&mut self.draft.jpeg_quality, 40..=100).suffix(" %"));
                });
            }

            ui.add_space(10.0);
            let mut keep = self.draft.keep_days > 0;
            let mut days = if keep { self.draft.keep_days } else { 30 };
            ui.horizontal(|ui| {
                ui.checkbox(&mut keep, "Move shots older than");
                ui.add_enabled(keep, egui::DragValue::new(&mut days).range(1..=3650));
                ui.label("days to the recycle bin");
            });
            self.draft.keep_days = if keep { days } else { 0 };
            if keep {
                ui.label(
                    RichText::new(
                        "    Checked at start and once an hour. This covers every image in \
                         the folder above, also ones Visura did not take.",
                    )
                    .color(palette.muted)
                    .size(11.5),
                );
            }
        });
    }

    fn after_capture_section(&mut self, ui: &mut egui::Ui, palette: &theme::Palette) {
        section(ui, palette, "After a capture", |ui| {
            ui.checkbox(&mut self.draft.after.copy_image, "Copy the image");
            ui.checkbox(&mut self.draft.after.copy_path, "Copy the file path");
            ui.checkbox(
                &mut self.draft.after.open_folder,
                "Show it in the file manager",
            );
            ui.add_space(6.0);
            ui.checkbox(
                &mut self.draft.after.delete_on_exit,
                "Delete this session's screenshots when Visura quits",
            );
            ui.label(
                RichText::new(
                    "    For shots that only exist to be pasted once. They go to the \
                     recycle bin, so a mistake can still be undone.",
                )
                .color(palette.muted)
                .size(11.5),
            );
        });
    }

    fn shortcut_section(&mut self, ui: &mut egui::Ui, palette: &theme::Palette) {
        section(ui, palette, "Shortcuts", |ui| {
            let mut keys = self.draft.hotkeys.clone();
            let mut recording = self.recording;
            let mut stop = false;

            for (action, value) in [
                (Action::Region, &mut keys.region),
                (Action::Window, &mut keys.window),
                (Action::Fullscreen, &mut keys.fullscreen),
            ] {
                let armed = recording == Some(action);
                labelled(ui, action.label(), |ui| {
                    let text = crate::ui::shortcut_text(value, armed);
                    if ui
                        .add_sized([170.0, 26.0], egui::Button::new(text).selected(armed))
                        .on_hover_text("Click, then press the key combination")
                        .clicked()
                    {
                        recording = if armed { None } else { Some(action) };
                        stop |= armed;
                    }
                    if ui
                        .add_enabled(!value.is_empty(), egui::Button::new("Clear"))
                        .on_hover_text("Leave this action without a shortcut")
                        .clicked()
                    {
                        value.clear();
                        recording = None;
                        stop = true;
                    }
                });

                if armed {
                    match record(ui) {
                        Recorded::Nothing => {}
                        Recorded::Cancelled => {
                            recording = None;
                            stop = true;
                        }
                        Recorded::Shortcut(shortcut) => {
                            *value = shortcut;
                            recording = None;
                            stop = true;
                        }
                    }
                }
            }

            self.draft.hotkeys = keys;
            if recording != self.recording {
                self.recording = recording;
                if recording.is_some() {
                    // Otherwise the running registration swallows the very key
                    // the user is trying to record.
                    self.hotkeys.suspend();
                }
            }
            if stop && self.recording.is_none() {
                let bound = self.config.hotkeys.clone();
                self.hotkeys.apply(&bound);
            }

            ui.add_space(6.0);
            ui.label(
                RichText::new("An action with no shortcut can still be started from the sidebar.")
                    .color(palette.muted)
                    .size(11.5),
            );

            if !self.hotkeys.problems.is_empty() {
                ui.add_space(8.0);
                for problem in &self.hotkeys.problems {
                    ui.label(
                        RichText::new(format!("• {problem}"))
                            .color(palette.danger)
                            .size(11.5),
                    );
                }
            }
        });
    }

    fn overlay_section(&mut self, ui: &mut egui::Ui, palette: &theme::Palette) {
        section(ui, palette, "Selection overlay", |ui| {
            ui.checkbox(
                &mut self.draft.overlay.detect_windows,
                "Outline the window under the cursor",
            );
            ui.add_enabled_ui(self.draft.overlay.detect_windows, |ui| {
                ui.checkbox(
                    &mut self.draft.overlay.panes_first,
                    "Click takes the pane under the cursor, such as a web page without the browser",
                );
                ui.label(
                    RichText::new(
                        "Hold Ctrl in the overlay to switch between pane and whole window.",
                    )
                    .color(palette.muted)
                    .size(11.5),
                );
            });
            ui.checkbox(&mut self.draft.overlay.crosshair, "Crosshair");
            ui.checkbox(
                &mut self.draft.overlay.magnifier,
                "Magnifier with colour value",
            );
            ui.checkbox(
                &mut self.draft.overlay.show_hints,
                "Hint line at the bottom",
            );
            ui.checkbox(
                &mut self.draft.overlay.hide_self,
                "Hide the Visura window while capturing",
            );
            ui.add_space(6.0);
            labelled(ui, "Dimming", |ui| {
                ui.add(
                    egui::Slider::new(&mut self.draft.overlay.dim, 0.0..=0.9)
                        .custom_formatter(|v, _| format!("{:.0} %", v * 100.0)),
                );
            });
        });
    }

    fn appearance_section(&mut self, ui: &mut egui::Ui, palette: &theme::Palette) {
        section(ui, palette, "Appearance and startup", |ui| {
            labelled(ui, "Theme", |ui| {
                ui.horizontal(|ui| {
                    for theme in Theme::ALL {
                        ui.selectable_value(&mut self.draft.ui.theme, theme, theme.label());
                    }
                });
            });
            if self.draft.ui.theme == Theme::Black {
                ui.label(
                    RichText::new("    Pure black, so an OLED panel switches those pixels off.")
                        .color(palette.muted)
                        .size(11.5),
                );
            }
            labelled(ui, "Accent", |ui| {
                ui.horizontal(|ui| {
                    for accent in Accent::ALL {
                        let [r, g, b] = accent.rgb();
                        let selected = self.draft.ui.accent == accent;
                        let (rect, response) =
                            ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::click());
                        let painter = ui.painter();
                        painter.rect_filled(
                            rect,
                            egui::CornerRadius::same(11),
                            egui::Color32::from_rgb(r, g, b),
                        );
                        if selected {
                            painter.rect_stroke(
                                rect.expand(2.0),
                                egui::CornerRadius::same(13),
                                egui::Stroke::new(1.5, palette.text),
                                egui::StrokeKind::Outside,
                            );
                        }
                        if response.on_hover_text(accent.label()).clicked() {
                            self.draft.ui.accent = accent;
                        }
                    }
                });
            });
            labelled(ui, "Tile size", |ui| {
                ui.add(
                    egui::Slider::new(&mut self.draft.ui.thumb_size, 120.0..=280.0)
                        .custom_formatter(|v, _| format!("{v:.0} px")),
                );
            });
            labelled(ui, "Recent shown", |ui| {
                ui.add(egui::Slider::new(&mut self.draft.ui.recent_count, 5..=60));
            });

            ui.add_space(8.0);
            ui.checkbox(&mut self.draft.ui.confirm_delete, "Ask before deleting");
            ui.checkbox(&mut self.draft.autostart, "Start with the system");
            if cfg!(windows) {
                ui.checkbox(
                    &mut self.draft.ui.close_to_tray,
                    "Closing only hides Visura in the notification area",
                );
                ui.checkbox(&mut self.draft.ui.start_hidden, "Start without a window");
            }
        });
    }

    // ---------------------------------------------------------- helpers ----

    /// Show what the current settings produce, using a fixed example moment so
    /// the line does not flicker once a second.
    fn name_preview(&self) -> String {
        let ctx = naming::Context {
            time: LocalTime {
                year: 2026,
                month: 9,
                day: 21,
                hour: 14,
                minute: 33,
                second: 12,
                millis: 480,
            },
            window_title: "Example window",
            app: "firefox",
            width: 1280,
            height: 720,
        };
        // A fixed example name, so the preview does not reshuffle itself on
        // every frame the way a real random name would.
        let pattern = if self.draft.anonymous_names {
            "q7x2m9k4d1bz"
        } else {
            &self.draft.filename
        };
        naming::target_path(
            &self.draft.folder,
            &self.draft.subfolder,
            pattern,
            self.draft.format.extension(),
            false,
            &ctx,
        )
        .display()
        .to_string()
    }
}

/// What the shortcut recorder saw this frame.
enum Recorded {
    Nothing,
    Cancelled,
    Shortcut(String),
}

/// Read the next key press as a shortcut.
///
/// PrintScreen, Pause and ScrollLock have no egui representation, and neither
/// has the Super key, so those come from the platform layer.
fn record(ui: &egui::Ui) -> Recorded {
    let modifiers = ui.input(|i| i.modifiers);
    let meta = crate::platform::super_key_down();

    if let Some(name) = crate::platform::poll_special_key() {
        return Recorded::Shortcut(hotkeys::join(
            modifiers.ctrl,
            modifiers.alt,
            modifiers.shift,
            meta,
            name,
        ));
    }

    ui.input(|i| {
        for event in &i.events {
            let egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } = event
            else {
                continue;
            };
            if *key == egui::Key::Escape {
                return Recorded::Cancelled;
            }
            if let Some(name) = hotkeys::name_of_egui_key(*key) {
                return Recorded::Shortcut(hotkeys::join(
                    modifiers.ctrl,
                    modifiers.alt,
                    modifiers.shift,
                    meta,
                    name,
                ));
            }
        }
        Recorded::Nothing
    })
}

fn section(
    ui: &mut egui::Ui,
    palette: &theme::Palette,
    title: &str,
    body: impl FnOnce(&mut egui::Ui),
) {
    ui.add_space(14.0);
    ui.label(RichText::new(title).size(13.5).color(palette.text).strong());
    ui.add_space(2.0);
    let width = ui.available_width();
    egui::Frame::default()
        .fill(palette.panel)
        .corner_radius(egui::CornerRadius::same(8))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            // Every section is the same width, otherwise the page looks like
            // a pile of differently sized cards.
            ui.set_min_width(width - 24.0);
            body(ui);
        });
}

/// A label of fixed width followed by a widget, so the rows line up.
fn labelled(ui: &mut egui::Ui, label: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(116.0, ui.spacing().interact_size.y),
            Layout::left_to_right(Align::Center),
            |ui| {
                ui.label(label);
            },
        );
        body(ui);
    });
}
