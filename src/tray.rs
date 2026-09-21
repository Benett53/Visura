//! Notification area icon.
//!
//! Windows only. On Linux a tray icon means pulling in GTK and
//! libayatana-appindicator, which is more weight than the feature is worth
//! here; the window simply stays open instead.

/// Without a tray these are never produced, but the app still matches on
/// them, so the enum stays whole on every platform.
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Region,
    Window,
    Fullscreen,
    Show,
    Settings,
    Quit,
}

#[cfg(windows)]
pub use self::windows_tray::Tray;

#[cfg(not(windows))]
pub struct Tray;

#[cfg(not(windows))]
impl Tray {
    pub fn new(_ctx: egui::Context) -> Option<Self> {
        None
    }

    pub fn poll(&self) -> Vec<Action> {
        Vec::new()
    }
}

#[cfg(windows)]
mod windows_tray {
    use std::sync::{Arc, Mutex};

    use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    use super::Action;

    /// Which menu entry is which, shared with the handlers that run on the
    /// system's own thread.
    struct Ids {
        region: MenuId,
        window: MenuId,
        fullscreen: MenuId,
        history: MenuId,
        settings: MenuId,
        quit: MenuId,
    }

    pub struct Tray {
        _icon: tray_icon::TrayIcon,
        pending: Arc<Mutex<Vec<Action>>>,
    }

    impl Tray {
        /// The context is only used to wake the program up when the tray is
        /// clicked. Without that the click sits in a queue until something
        /// else happens to draw a frame.
        pub fn new(ctx: egui::Context) -> Option<Self> {
            let (rgba, width, height) = crate::icon::rgba()?;
            let icon = Icon::from_rgba(rgba, width, height).ok()?;

            let menu = Menu::new();
            let region = MenuItem::new("Capture region", true, None);
            let window = MenuItem::new("Capture window", true, None);
            let fullscreen = MenuItem::new("Capture screen", true, None);
            let history = MenuItem::new("Open Visura", true, None);
            let settings = MenuItem::new("Settings", true, None);
            let quit = MenuItem::new("Quit", true, None);

            menu.append(&region).ok()?;
            menu.append(&window).ok()?;
            menu.append(&fullscreen).ok()?;
            menu.append(&PredefinedMenuItem::separator()).ok()?;
            menu.append(&history).ok()?;
            menu.append(&settings).ok()?;
            menu.append(&PredefinedMenuItem::separator()).ok()?;
            menu.append(&quit).ok()?;

            let ids = Ids {
                region: region.id().clone(),
                window: window.id().clone(),
                fullscreen: fullscreen.id().clone(),
                history: history.id().clone(),
                settings: settings.id().clone(),
                quit: quit.id().clone(),
            };

            let tray = TrayIconBuilder::new()
                .with_tooltip("Visura")
                .with_icon(icon)
                .with_menu(Box::new(menu))
                .build()
                .ok()?;

            let pending: Arc<Mutex<Vec<Action>>> = Arc::new(Mutex::new(Vec::new()));

            {
                let pending = pending.clone();
                let ctx = ctx.clone();
                MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                    let id = &event.id;
                    let action = if id == &ids.region {
                        Action::Region
                    } else if id == &ids.window {
                        Action::Window
                    } else if id == &ids.fullscreen {
                        Action::Fullscreen
                    } else if id == &ids.history {
                        Action::Show
                    } else if id == &ids.settings {
                        Action::Settings
                    } else if id == &ids.quit {
                        Action::Quit
                    } else {
                        return;
                    };
                    if let Ok(mut queue) = pending.lock() {
                        queue.push(action);
                    }
                    ctx.request_repaint();
                }));
            }

            {
                let pending = pending.clone();
                TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
                    // A left click opens the window; the menu is on right
                    // click and arrives through MenuEvent above.
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        if let Ok(mut queue) = pending.lock() {
                            queue.push(Action::Show);
                        }
                        ctx.request_repaint();
                    }
                }));
            }

            Some(Self {
                _icon: tray,
                pending,
            })
        }

        /// Take everything the handlers collected since the last call.
        pub fn poll(&self) -> Vec<Action> {
            match self.pending.lock() {
                Ok(mut queue) => std::mem::take(&mut *queue),
                Err(_) => Vec::new(),
            }
        }
    }
}
