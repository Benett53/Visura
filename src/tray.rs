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
    pub fn new() -> Option<Self> {
        None
    }

    pub fn poll(&self) -> Vec<Action> {
        Vec::new()
    }
}

#[cfg(windows)]
mod windows_tray {
    use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

    use super::Action;

    pub struct Tray {
        _icon: tray_icon::TrayIcon,
        region: MenuId,
        window: MenuId,
        fullscreen: MenuId,
        history: MenuId,
        settings: MenuId,
        quit: MenuId,
    }

    impl Tray {
        pub fn new() -> Option<Self> {
            let (rgba, width, height) = crate::icon::rgba()?;
            let icon = Icon::from_rgba(rgba, width, height).ok()?;

            let menu = Menu::new();
            let region = MenuItem::new("Bereich aufnehmen", true, None);
            let window = MenuItem::new("Fenster aufnehmen", true, None);
            let fullscreen = MenuItem::new("Ganzer Bildschirm", true, None);
            let history = MenuItem::new("Historie", true, None);
            let settings = MenuItem::new("Einstellungen", true, None);
            let quit = MenuItem::new("Beenden", true, None);

            menu.append(&region).ok()?;
            menu.append(&window).ok()?;
            menu.append(&fullscreen).ok()?;
            menu.append(&PredefinedMenuItem::separator()).ok()?;
            menu.append(&history).ok()?;
            menu.append(&settings).ok()?;
            menu.append(&PredefinedMenuItem::separator()).ok()?;
            menu.append(&quit).ok()?;

            let tray = TrayIconBuilder::new()
                .with_tooltip("Visura")
                .with_icon(icon)
                .with_menu(Box::new(menu))
                .build()
                .ok()?;

            Some(Self {
                _icon: tray,
                region: region.id().clone(),
                window: window.id().clone(),
                fullscreen: fullscreen.id().clone(),
                history: history.id().clone(),
                settings: settings.id().clone(),
                quit: quit.id().clone(),
            })
        }

        pub fn poll(&self) -> Vec<Action> {
            let mut actions = Vec::new();
            while let Ok(event) = MenuEvent::receiver().try_recv() {
                let id = &event.id;
                let action = if id == &self.region {
                    Action::Region
                } else if id == &self.window {
                    Action::Window
                } else if id == &self.fullscreen {
                    Action::Fullscreen
                } else if id == &self.history {
                    Action::Show
                } else if id == &self.settings {
                    Action::Settings
                } else if id == &self.quit {
                    Action::Quit
                } else {
                    continue;
                };
                actions.push(action);
            }
            while let Ok(event) = TrayIconEvent::receiver().try_recv() {
                // A left click opens the history; the menu is on right click
                // and is delivered through MenuEvent above.
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    actions.push(Action::Show);
                }
            }
            actions
        }
    }
}
