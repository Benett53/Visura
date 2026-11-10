//! Notification area icon.
//!
//! Windows uses `tray-icon`. Linux speaks StatusNotifierItem over D-Bus
//! through `ksni`, which KDE Plasma shows natively and GNOME with the
//! AppIndicator extension; no GTK is involved. Where nothing listens for
//! such items there is no icon, and closing the window quits as before.

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

#[cfg(unix)]
pub use self::unix_tray::Tray;

#[cfg(unix)]
mod unix_tray {
    use std::sync::{Arc, Mutex};

    use ksni::blocking::TrayMethods as _;
    use ksni::menu::{MenuItem, StandardItem};

    use super::Action;

    pub struct Tray {
        // Dropping the handle would take the icon away again.
        _handle: ksni::blocking::Handle<Item>,
        pending: Arc<Mutex<Vec<Action>>>,
    }

    /// What the desktop asks about and calls into, on a thread of `ksni`.
    struct Item {
        icons: Vec<ksni::Icon>,
        pending: Arc<Mutex<Vec<Action>>>,
        ctx: egui::Context,
    }

    impl Item {
        /// Queue an action and wake the program, which sleeps while hidden.
        fn send(&self, action: Action) {
            if let Ok(mut queue) = self.pending.lock() {
                queue.push(action);
            }
            self.ctx.request_repaint();
        }

        fn entry(label: &str, action: Action) -> MenuItem<Self> {
            StandardItem {
                label: label.into(),
                activate: Box::new(move |item: &mut Self| item.send(action)),
                ..Default::default()
            }
            .into()
        }
    }

    impl ksni::Tray for Item {
        fn id(&self) -> String {
            "visura".into()
        }

        fn title(&self) -> String {
            "Visura".into()
        }

        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            self.icons.clone()
        }

        fn tool_tip(&self) -> ksni::ToolTip {
            ksni::ToolTip {
                title: "Visura".into(),
                ..Default::default()
            }
        }

        /// Left click.
        fn activate(&mut self, _x: i32, _y: i32) {
            self.send(Action::Show);
        }

        /// Middle click.
        fn secondary_activate(&mut self, _x: i32, _y: i32) {
            self.send(Action::Region);
        }

        fn menu(&self) -> Vec<MenuItem<Self>> {
            vec![
                Self::entry("Capture region", Action::Region),
                Self::entry("Capture window", Action::Window),
                Self::entry("Capture screen", Action::Fullscreen),
                MenuItem::Separator,
                Self::entry("Open Visura", Action::Show),
                Self::entry("Settings", Action::Settings),
                MenuItem::Separator,
                Self::entry("Quit", Action::Quit),
            ]
        }
    }

    /// The icon in the sizes panels commonly ask for, as ARGB32 in network
    /// byte order, which is what the StatusNotifierItem specification wants.
    fn icons() -> Vec<ksni::Icon> {
        [22, 32, 48, 64]
            .into_iter()
            .filter_map(crate::icon::rgba_sized)
            .map(|(rgba, width, height)| ksni::Icon {
                width: width as i32,
                height: height as i32,
                data: rgba
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .flat_map(|p| [p[3], p[0], p[1], p[2]])
                    .collect(),
            })
            .collect()
    }

    impl Tray {
        /// `None` where no desktop component shows such icons.
        pub fn new(ctx: egui::Context) -> Option<Self> {
            let pending: Arc<Mutex<Vec<Action>>> = Arc::new(Mutex::new(Vec::new()));
            let item = Item {
                icons: icons(),
                pending: pending.clone(),
                ctx,
            };
            let handle = item.spawn().ok()?;
            Some(Self {
                _handle: handle,
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
