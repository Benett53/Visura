//! Settings, stored as TOML and nothing else.
//!
//! The file is meant to be readable and editable by hand. Unknown keys are
//! ignored and missing keys fall back to the default, so an older or newer
//! file never stops the program from starting.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::platform;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Png,
    Jpeg,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Png => "png",
            Format::Jpeg => "jpg",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Format::Png => "PNG (lossless)",
            Format::Jpeg => "JPEG (smaller)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Dark,
    /// Pure black, for OLED panels where black pixels are switched off.
    Black,
    Light,
}

impl Theme {
    pub const ALL: [Theme; 3] = [Theme::Dark, Theme::Black, Theme::Light];

    pub fn label(self) -> &'static str {
        match self {
            Theme::Dark => "Dark",
            Theme::Black => "Black",
            Theme::Light => "Light",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Accent {
    Grey,
    Blue,
    Teal,
    Green,
    Amber,
    Red,
    Purple,
}

impl Accent {
    pub const ALL: [Accent; 7] = [
        Accent::Grey,
        Accent::Blue,
        Accent::Teal,
        Accent::Green,
        Accent::Amber,
        Accent::Red,
        Accent::Purple,
    ];

    pub fn rgb(self) -> [u8; 3] {
        match self {
            Accent::Grey => [0x9a, 0xa0, 0xa8],
            Accent::Blue => [0x4c, 0x8d, 0xf6],
            Accent::Teal => [0x2f, 0xb3, 0xa8],
            Accent::Green => [0x5a, 0xb5, 0x6d],
            Accent::Amber => [0xd8, 0x9b, 0x3c],
            Accent::Red => [0xd9, 0x5f, 0x5f],
            Accent::Purple => [0x9a, 0x7a, 0xe0],
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Accent::Grey => "Grey",
            Accent::Blue => "Blue",
            Accent::Teal => "Teal",
            Accent::Green => "Green",
            Accent::Amber => "Amber",
            Accent::Red => "Red",
            Accent::Purple => "Purple",
        }
    }
}

/// An empty string means the action has no shortcut.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    pub region: String,
    pub window: String,
    pub fullscreen: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            // Region is the one people reach for; the other two start unbound
            // so nothing is taken from the system without being asked for.
            region: "PrintScreen".into(),
            window: String::new(),
            fullscreen: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Overlay {
    /// How dark the area outside the selection gets, 0.0 to 1.0.
    pub dim: f32,
    pub magnifier: bool,
    pub crosshair: bool,
    /// Outline the window under the cursor and capture it on a plain click.
    pub detect_windows: bool,
    pub show_hints: bool,
    /// Hide the Visura window before capturing so it stays out of the shot.
    pub hide_self: bool,
}

impl Default for Overlay {
    fn default() -> Self {
        Self {
            dim: 0.55,
            magnifier: false,
            crosshair: true,
            detect_windows: true,
            show_hints: false,
            hide_self: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AfterCapture {
    pub copy_image: bool,
    pub copy_path: bool,
    pub open_folder: bool,
}

impl Default for AfterCapture {
    fn default() -> Self {
        Self {
            // The clipboard is the reason most screenshots are taken at all.
            copy_image: true,
            copy_path: false,
            open_folder: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ui {
    pub theme: Theme,
    pub accent: Accent,
    /// Edge length of a history tile in points.
    pub thumb_size: f32,
    /// How many shots the main window shows before the full history is needed.
    pub recent_count: usize,
    pub start_hidden: bool,
    pub close_to_tray: bool,
    pub confirm_delete: bool,
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            accent: Accent::Grey,
            thumb_size: 168.0,
            recent_count: 15,
            start_hidden: false,
            close_to_tray: false,
            confirm_delete: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Root of the screenshot library.
    pub folder: PathBuf,
    /// Folder pattern below the root; empty means everything in one folder.
    pub subfolder: String,
    pub filename: String,
    /// Replace the name pattern with a random string, so a file name carries
    /// no hint about what was captured or when.
    pub anonymous_names: bool,
    pub format: Format,
    pub jpeg_quality: u8,
    pub autostart: bool,
    pub after: AfterCapture,
    pub hotkeys: Hotkeys,
    pub overlay: Overlay,
    pub ui: Ui,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            folder: platform::default_screenshot_dir(),
            // One folder per month keeps directory listings usable for years
            // without burying this morning behind three clicks.
            subfolder: "%Y-%m".into(),
            // The program the window belongs to, then the date. A file name
            // that says what it is beats one that only says when it was.
            filename: "%app_%Y-%m-%d".into(),
            anonymous_names: false,
            format: Format::Png,
            jpeg_quality: 90,
            autostart: false,
            after: AfterCapture::default(),
            hotkeys: Hotkeys::default(),
            overlay: Overlay::default(),
            ui: Ui::default(),
        }
    }
}

impl Config {
    pub fn path() -> PathBuf {
        if let Some(custom) = std::env::var_os("VISURA_CONFIG") {
            return PathBuf::from(custom);
        }
        platform::config_dir().join("config.toml")
    }

    /// Never fails. A broken file is reported and the defaults are used, so a
    /// typo in the config cannot lock anyone out of their own screenshots.
    pub fn load() -> (Self, Option<String>) {
        let path = Self::path();
        let Ok(text) = std::fs::read_to_string(&path) else {
            return (Self::default(), None);
        };
        match toml::from_str::<Config>(&text) {
            Ok(config) => (config.sanitised(), None),
            Err(e) => (
                Self::default(),
                Some(format!("{} could not be read: {e}", path.display())),
            ),
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let text = toml::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, text).map_err(|e| e.to_string())
    }

    /// Clamp values that would otherwise produce a broken window or unreadable
    /// files if someone edits the file by hand.
    fn sanitised(mut self) -> Self {
        self.overlay.dim = self.overlay.dim.clamp(0.0, 0.9);
        self.jpeg_quality = self.jpeg_quality.clamp(20, 100);
        self.ui.thumb_size = self.ui.thumb_size.clamp(96.0, 320.0);
        self.ui.recent_count = self.ui.recent_count.clamp(1, 200);
        if self.filename.trim().is_empty() {
            self.filename = Config::default().filename;
        }
        if self.folder.as_os_str().is_empty() {
            self.folder = platform::default_screenshot_dir();
        }
        // An empty shortcut means "not bound" and is left alone. One that
        // cannot be parsed would silently do nothing, so it is reset.
        let defaults = Hotkeys::default();
        for (value, fallback) in [
            (&mut self.hotkeys.region, defaults.region),
            (&mut self.hotkeys.window, defaults.window),
            (&mut self.hotkeys.fullscreen, defaults.fullscreen),
        ] {
            if !value.is_empty() && !crate::hotkeys::is_valid(value) {
                *value = fallback;
            }
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip_through_toml() {
        let config = Config::default();
        let text = toml::to_string_pretty(&config).unwrap();
        let back: Config = toml::from_str(&text).unwrap();
        assert_eq!(config, back);
    }

    #[test]
    fn a_partial_file_keeps_the_other_defaults() {
        let parsed: Config = toml::from_str("jpeg_quality = 50\n").unwrap();
        assert_eq!(parsed.jpeg_quality, 50);
        assert_eq!(parsed.format, Format::Png);
        assert_eq!(parsed.hotkeys, Hotkeys::default());
    }

    #[test]
    fn nonsense_values_are_clamped() {
        let parsed: Config = toml::from_str("jpeg_quality = 250\n").unwrap();
        assert_eq!(parsed.sanitised().jpeg_quality, 100);
    }

    #[test]
    fn a_broken_shortcut_falls_back_to_the_default() {
        let parsed: Config = toml::from_str("[hotkeys]\nregion = \"Ctrl+Nonsense\"\n").unwrap();
        assert_eq!(parsed.sanitised().hotkeys.region, Hotkeys::default().region);
    }

    #[test]
    fn an_empty_shortcut_stays_empty() {
        let parsed: Config = toml::from_str("[hotkeys]\nregion = \"\"\n").unwrap();
        assert_eq!(parsed.sanitised().hotkeys.region, "");
    }

    #[test]
    fn only_region_is_bound_out_of_the_box() {
        let keys = Hotkeys::default();
        assert!(!keys.region.is_empty());
        assert!(keys.window.is_empty());
        assert!(keys.fullscreen.is_empty());
    }
}
