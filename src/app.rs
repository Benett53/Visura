//! The application shell: window, state, and the path a capture takes from a
//! hotkey to a file in the library.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::capture::{self, Mode};
use crate::config::Config;
use crate::hotkeys;
use crate::library::{self, Shot};
use crate::overlay::{self, Overlay};
use crate::platform::{self, LocalTime, WindowInfo};
use crate::thumbs::Thumbs;
use crate::{clipboard, tray, ui};

/// How long to keep asking whether the window has really gone before giving
/// up and capturing anyway.
const HIDE_TIMEOUT: Duration = Duration::from_millis(400);
/// A last pause once the window reports itself gone, for the compositor to
/// finish putting the pixels underneath back.
const HIDE_SETTLE: Duration = Duration::from_millis(40);
const TOAST_LIFETIME: Duration = Duration::from_secs(5);

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Page {
    Recent,
    Settings,
}

pub struct Toast {
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

/// A capture in flight. The window has to be out of the way before the screen
/// is read, and that takes a frame or two.
struct Pending {
    mode: Mode,
    /// The window was on screen when the capture was asked for, so it belongs
    /// back on screen afterwards. This is not the same as having hidden it:
    /// with hiding switched off the window still has to come back.
    was_visible: bool,
    /// Whether this capture hid the window and has to wait for it to go.
    hiding: bool,
    /// Captured before hiding, because afterwards the front window is a
    /// different one.
    front_window: Option<WindowInfo>,
}

pub struct App {
    pub config: Config,
    /// The settings being edited. Applied on save, discarded on cancel.
    pub draft: Config,
    pub page: Page,

    pub shots: Vec<Shot>,
    /// Indices into `shots` that match the search, newest first.
    pub view: Vec<usize>,
    /// Headings and the range of `view` they cover.
    pub groups: Vec<(String, std::ops::Range<usize>)>,
    pub selection: HashSet<PathBuf>,
    pub last_clicked: Option<(crate::ui::history::List, usize)>,
    pub search: String,
    pub renaming: Option<(PathBuf, String)>,
    pub pending_delete: Vec<PathBuf>,

    pub thumbs: Thumbs,
    pub hotkeys: hotkeys::Manager,
    pub toast: Option<Toast>,
    pub today: LocalTime,

    overlay: Option<Overlay>,
    pending: Option<Pending>,
    /// Where the window sat before it became the overlay: outer position
    /// and inner size, both in points.
    last_geometry: Option<(egui::Pos2, egui::Vec2)>,
    /// Whether the window should be on screen once the overlay is done.
    overlay_restore: bool,
    /// The window is put in the middle of the screen once, on the first frame,
    /// when its real size is known.
    centred: bool,
    /// The full history lives in a window of its own.
    pub history_open: bool,
    /// Which shortcut is currently listening for a key press.
    pub recording: Option<hotkeys::Action>,
    tray: Option<tray::Tray>,
    show_requested: Arc<AtomicBool>,
    visible: bool,
    quitting: bool,
    startup_note: Option<String>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, start_hidden: bool) -> Self {
        // The exact window handle, rather than one found by guesswork later.
        #[cfg(windows)]
        {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(handle) = cc.window_handle()
                && let RawWindowHandle::Win32(win32) = handle.as_raw()
            {
                platform::set_main_window(win32.hwnd.get());
            }
        }

        let (config, problem) = Config::load();
        ui::theme::apply(&cc.egui_ctx, config.ui.theme, config.ui.accent);

        let mut hotkeys = hotkeys::Manager::new();
        hotkeys.apply(&config.hotkeys);

        let show_requested = Arc::new(AtomicBool::new(false));
        if let Some(guard) = crate::INSTANCE.lock().ok().and_then(|mut g| g.take()) {
            let flag = show_requested.clone();
            let ctx = cc.egui_ctx.clone();
            platform::spawn_show_listener(guard, move || {
                flag.store(true, Ordering::SeqCst);
                ctx.request_repaint();
            });
        }

        let tray = tray::Tray::new();
        let shots = library::scan(&config.folder);

        let mut app = Self {
            draft: config.clone(),
            config,
            page: Page::Recent,
            shots,
            view: Vec::new(),
            groups: Vec::new(),
            selection: HashSet::new(),
            last_clicked: None,
            search: String::new(),
            renaming: None,
            pending_delete: Vec::new(),
            thumbs: Thumbs::new(),
            hotkeys,
            toast: None,
            today: platform::local_now(),
            overlay: None,
            pending: None,
            last_geometry: None,
            overlay_restore: false,
            centred: false,
            history_open: false,
            recording: None,
            tray,
            show_requested,
            visible: !start_hidden,
            quitting: false,
            startup_note: problem,
        };
        app.rebuild_view();
        app
    }

    // -------------------------------------------------------- library ----

    pub fn refresh(&mut self) {
        self.shots = library::scan(&self.config.folder);
        // Anything that vanished should not stay selected.
        let present: HashSet<&PathBuf> = self.shots.iter().map(|s| &s.path).collect();
        self.selection.retain(|p| present.contains(p));
        self.rebuild_view();
    }

    pub fn rebuild_view(&mut self) {
        let needle = self.search.trim().to_lowercase();
        self.view = self
            .shots
            .iter()
            .enumerate()
            .filter(|(_, shot)| needle.is_empty() || shot.name.to_lowercase().contains(&needle))
            .map(|(i, _)| i)
            .collect();

        self.groups.clear();
        let mut start = 0;
        let mut current_day = None;
        for (position, &index) in self.view.iter().enumerate() {
            let day = self.shots[index].taken.day_number();
            if current_day != Some(day) {
                if let Some(previous) = current_day {
                    let _ = previous;
                    self.groups.push((
                        library::day_label(self.shots[self.view[start]].taken, self.today),
                        start..position,
                    ));
                }
                current_day = Some(day);
                start = position;
            }
        }
        if current_day.is_some() && start < self.view.len() {
            self.groups.push((
                library::day_label(self.shots[self.view[start]].taken, self.today),
                start..self.view.len(),
            ));
        }
    }

    pub fn selected_paths(&self) -> Vec<PathBuf> {
        // Kept in library order so a multi-file drag arrives sorted.
        self.shots
            .iter()
            .filter(|s| self.selection.contains(&s.path))
            .map(|s| s.path.clone())
            .collect()
    }

    pub fn note(&mut self, text: impl Into<String>) {
        self.toast = Some(Toast {
            text: text.into(),
            error: false,
            at: Instant::now(),
        });
    }

    pub fn warn(&mut self, text: impl Into<String>) {
        self.toast = Some(Toast {
            text: text.into(),
            error: true,
            at: Instant::now(),
        });
    }

    // -------------------------------------------------------- actions ----

    pub fn request_capture(&mut self, mode: Mode) {
        if self.pending.is_some() || self.overlay.is_some() {
            return;
        }
        // Triggered from the hotkey, the front window is the one the user is
        // looking at. Triggered from a button it is Visura itself, which the
        // platform layer refuses to report, so the window behind it is meant.
        let front_window = platform::foreground_window()
            .or_else(|| platform::windows_in_z_order().into_iter().next());
        self.pending = Some(Pending {
            mode,
            was_visible: self.visible,
            hiding: self.visible && self.config.overlay.hide_self,
            front_window,
        });
    }

    pub fn delete_selected(&mut self) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        if self.config.ui.confirm_delete {
            self.pending_delete = paths;
            return;
        }
        self.perform_delete(paths);
    }

    pub fn perform_delete(&mut self, paths: Vec<PathBuf>) {
        let count = paths.len();
        match library::delete(&paths) {
            Ok(()) => {
                for path in &paths {
                    self.thumbs.forget(path);
                }
                self.selection.clear();
                library::prune_empty_dirs(&self.config.folder);
                self.refresh();
                self.note(if count == 1 {
                    "Moved to the recycle bin".to_string()
                } else {
                    format!("{count} images moved to the recycle bin")
                });
            }
            Err(e) => self.warn(format!("Deleting failed: {e}")),
        }
    }

    pub fn copy_image_of(&mut self, path: &PathBuf) {
        match image::open(path) {
            Ok(decoded) => {
                let rgba = decoded.to_rgba8();
                let image = platform::Image {
                    width: rgba.width(),
                    height: rgba.height(),
                    rgba: rgba.into_raw(),
                };
                match clipboard::copy_image(&image) {
                    Ok(()) => self.note("Image copied"),
                    Err(e) => self.warn(format!("Copying failed: {e}")),
                }
            }
            Err(e) => self.warn(format!("The image could not be read: {e}")),
        }
    }

    pub fn start_drag(&mut self) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        if let Err(e) = platform::start_file_drag(&paths) {
            self.warn(e);
        }
    }

    pub fn finish_rename(&mut self) {
        let Some((path, new_name)) = self.renaming.take() else {
            return;
        };
        let cleaned = crate::naming::sanitise(&new_name);
        if cleaned.is_empty() {
            return;
        }
        let extension = path
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let stem_matches = cleaned.ends_with(&extension) && !extension.is_empty();
        let target = path.with_file_name(if stem_matches {
            cleaned
        } else {
            format!("{cleaned}{extension}")
        });
        if target == path {
            return;
        }
        if target.exists() {
            self.warn("A file with that name already exists");
            return;
        }
        match std::fs::rename(&path, &target) {
            Ok(()) => {
                self.thumbs.forget(&path);
                self.selection.remove(&path);
                self.selection.insert(target);
                self.refresh();
            }
            Err(e) => self.warn(format!("Renaming failed: {e}")),
        }
    }

    pub fn apply_settings(&mut self) {
        let folder_changed = self.draft.folder != self.config.folder;
        let hotkeys_changed = self.draft.hotkeys != self.config.hotkeys;
        let theme_changed = self.draft.ui.theme != self.config.ui.theme
            || self.draft.ui.accent != self.config.ui.accent;
        let autostart_changed = self.draft.autostart != self.config.autostart;

        self.config = self.draft.clone();
        if let Err(e) = self.config.save() {
            self.warn(format!("The settings were not saved: {e}"));
            return;
        }
        if hotkeys_changed {
            let keys = self.config.hotkeys.clone();
            self.hotkeys.apply(&keys);
        }
        if autostart_changed && let Err(e) = platform::set_autostart(self.config.autostart) {
            self.warn(format!("Autostart could not be changed: {e}"));
            return;
        }
        if folder_changed {
            self.refresh();
        }
        let _ = theme_changed;
        self.note("Settings saved");
    }

    /// One time setup that needs a window to already exist: switch off the
    /// open and close animation, and put the window in the middle of the
    /// monitor the mouse is on.
    ///
    /// Both wait for the first frame, because the real window size is only
    /// known after the window manager has had its say.
    fn prepare_window(&mut self, ctx: &egui::Context) {
        let Some(size) = ctx.input(|i| i.viewport().outer_rect.map(|r| r.size())) else {
            return;
        };
        if size.x < 1.0 || size.y < 1.0 {
            return;
        }
        let area = platform::work_area_at_cursor();
        if area.is_empty() {
            self.centred = true;
            return;
        }
        let points = ctx.pixels_per_point().max(0.1);
        let (w, h) = (area.w as f32 / points, area.h as f32 / points);
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
            area.x as f32 / points + (w - size.x) / 2.0,
            area.y as f32 / points + (h - size.y) / 2.0,
        )));
        self.centred = true;
    }

    fn show_window(&mut self, ctx: &egui::Context) {
        self.visible = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
    }

    fn hide_window(&mut self, ctx: &egui::Context) {
        self.visible = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    }

    // --------------------------------------------------------- capture ----

    fn tick_capture(&mut self, ctx: &egui::Context) {
        let Some(pending) = &mut self.pending else {
            return;
        };

        // The hide command only reaches the window manager at the end of this
        // pass, so hiding always costs one frame before anything else happens.
        if pending.hiding && self.visible {
            self.visible = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            ctx.request_repaint();
            return;
        }

        let Some(pending) = self.pending.take() else {
            return;
        };
        if pending.hiding {
            // Wait for the window to actually be gone instead of guessing how
            // long that takes. Guessing is what left a half transparent ghost
            // of the window in the shot.
            let deadline = std::time::Instant::now() + HIDE_TIMEOUT;
            while platform::own_window_is_visible() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            std::thread::sleep(HIDE_SETTLE);
        }

        let (image, screen) = match capture::grab_screen() {
            Ok(result) => result,
            Err(e) => {
                self.warn(format!("The screen could not be read: {e}"));
                if pending.was_visible {
                    self.show_window(ctx);
                }
                return;
            }
        };

        match pending.mode {
            Mode::Fullscreen => {
                self.store(&image, None);
                if pending.was_visible {
                    self.show_window(ctx);
                }
            }
            Mode::ActiveWindow => {
                match pending.front_window {
                    Some(window) => {
                        let local = crate::platform::Rect::new(
                            window.rect.x - screen.x,
                            window.rect.y - screen.y,
                            window.rect.w,
                            window.rect.h,
                        );
                        let cropped = image.crop(local);
                        self.store(&cropped, Some(&window));
                    }
                    None => self.warn("No window is in front"),
                }
                if pending.was_visible {
                    self.show_window(ctx);
                }
            }
            Mode::Region => {
                let windows = platform::windows_in_z_order();
                let overlay = Overlay::new(
                    image,
                    screen,
                    windows,
                    self.config.overlay.clone(),
                    self.config.ui.accent.rgb(),
                );
                self.enter_overlay(ctx, overlay, pending.was_visible);
            }
        }
    }

    fn store(&mut self, image: &platform::Image, window: Option<&WindowInfo>) {
        match capture::store(image, &self.config, window) {
            Ok(outcome) => {
                for note in &outcome.notes {
                    self.warn(note.clone());
                }
                let name = outcome
                    .path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if self.toast.is_none() || !self.toast.as_ref().unwrap().error {
                    self.note(format!("{name}  ·  {} × {}", outcome.width, outcome.height));
                }
                self.today = platform::local_now();
                self.refresh();
                self.selection.clear();
                self.selection.insert(outcome.path);
            }
            Err(e) => self.warn(e),
        }
    }

    // --------------------------------------------------------- overlay ----

    /// Turn the main window itself into the full screen overlay.
    ///
    /// A separate window would be tidier on paper, but eframe skips the whole
    /// UI pass for a viewport the platform reports as not visible, and a tray
    /// application is in exactly that state when the hotkey arrives. Reusing
    /// the one window keeps the overlay working from the tray either way, and
    /// saves a second GL surface.
    fn enter_overlay(&mut self, ctx: &egui::Context, overlay: Overlay, restore_visible: bool) {
        let screen = overlay.screen;
        let points = ctx.pixels_per_point().max(0.1);
        self.overlay = Some(overlay);
        self.overlay_restore = restore_visible;

        ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
            egui::WindowLevel::AlwaysOnTop,
        ));
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
            screen.x as f32 / points,
            screen.y as f32 / points,
        )));
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
            screen.w as f32 / points,
            screen.h as f32 / points,
        )));
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        self.visible = true;
        ctx.request_repaint();
    }

    fn leave_overlay(&mut self, ctx: &egui::Context) {
        self.overlay = None;
        self.pending = None;
        ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(
            egui::WindowLevel::Normal,
        ));
        ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(true));
        if let Some((position, size)) = self.last_geometry {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(position));
        }
        let visible = self.overlay_restore;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(visible));
        if visible {
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        }
        self.visible = visible;
        ctx.request_repaint();
    }

    fn overlay_ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let Some(mut overlay) = self.overlay.take() else {
            return;
        };
        let outcome = overlay.ui(ui);
        ctx.request_repaint();

        match outcome {
            overlay::Outcome::Pending => self.overlay = Some(overlay),
            overlay::Outcome::Cancelled => self.leave_overlay(&ctx),
            overlay::Outcome::Selected(region) => {
                let screen = overlay.screen;
                let cropped = overlay.image.crop(platform::Rect::new(
                    region.x - screen.x,
                    region.y - screen.y,
                    region.w,
                    region.h,
                ));
                let source = overlay
                    .window_at(region.x + region.w / 2, region.y + region.h / 2)
                    .cloned();
                self.leave_overlay(&ctx);
                self.store(&cropped, source.as_ref());
            }
        }
    }

    // ----------------------------------------------------------- input ----

    // ----------------------------------------------------------- input ----

    fn tick_hotkeys(&mut self, ctx: &egui::Context) {
        for action in self.hotkeys.poll() {
            let mode = match action {
                hotkeys::Action::Region => Mode::Region,
                hotkeys::Action::Window => Mode::ActiveWindow,
                hotkeys::Action::Fullscreen => Mode::Fullscreen,
            };
            self.request_capture(mode);
            ctx.request_repaint();
        }
    }

    fn tick_tray(&mut self, ctx: &egui::Context) {
        let Some(tray) = &self.tray else {
            return;
        };
        for action in tray.poll() {
            match action {
                tray::Action::Region => self.request_capture(Mode::Region),
                tray::Action::Window => self.request_capture(Mode::ActiveWindow),
                tray::Action::Fullscreen => self.request_capture(Mode::Fullscreen),
                tray::Action::Show => {
                    self.page = Page::Recent;
                    self.refresh();
                    self.show_window(ctx);
                }
                tray::Action::Settings => {
                    self.draft = self.config.clone();
                    self.page = Page::Settings;
                    self.show_window(ctx);
                }
                tray::Action::Quit => self.quitting = true,
            }
            ctx.request_repaint();
        }
    }
}

impl eframe::App for App {
    /// Runs even while the window is hidden, which is where a tray application
    /// spends most of its life.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.show_requested.swap(false, Ordering::SeqCst) {
            self.refresh();
            self.show_window(ctx);
        }

        self.tick_tray(ctx);
        self.tick_hotkeys(ctx);
        self.tick_capture(ctx);

        if self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        if !self.visible {
            // Hidden means no rendering at all, just a slow pulse so a hotkey
            // or a tray click is noticed within a fraction of a second.
            ctx.request_repaint_after(Duration::from_millis(120));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if let Some(note) = self.startup_note.take() {
            self.warn(note);
        }

        if self.overlay.is_some() {
            self.overlay_ui(ui);
            return;
        }

        if !self.centred {
            self.prepare_window(&ctx);
        }

        // Remembered every frame, so the overlay can put the window back
        // wherever the user last had it.
        // Not while a capture is in flight: the window is being moved and
        // hidden then, and remembering that would lose where it really lives.
        if self.pending.is_none() {
            ctx.input(|i| {
                if let (Some(outer), Some(inner)) =
                    (i.viewport().outer_rect, i.viewport().inner_rect)
                {
                    self.last_geometry = Some((outer.min, inner.size()));
                }
            });
        }

        if self.thumbs.collect(&ctx) {
            ctx.request_repaint();
        }

        // Closing the window with a tray icon present only puts it away;
        // without one there would be no way back, so it really quits.
        if ctx.input(|i| i.viewport().close_requested())
            && self.config.ui.close_to_tray
            && self.tray.is_some()
            && !self.quitting
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide_window(&ctx);
        }

        if let Some(toast) = &self.toast
            && toast.at.elapsed() > TOAST_LIFETIME
        {
            self.toast = None;
        }

        ui::shell(self, ui);

        if self.thumbs.is_busy() {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        let known: HashSet<u64> = self
            .shots
            .iter()
            .map(|s| crate::thumbs::key_for(&s.path, s.modified, s.bytes))
            .collect();
        crate::thumbs::sweep_cache(&known);
    }
}
