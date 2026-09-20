//! Global shortcuts.
//!
//! A shortcut is stored as text such as `Ctrl+Shift+PrintScreen`, so the
//! config file stays readable and a shortcut can be set without running the
//! program. An empty string means the action has no shortcut at all.

use std::collections::HashMap;

use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager};

use crate::config::Hotkeys;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Region,
    Window,
    Fullscreen,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Action::Region => "Region",
            Action::Window => "Window",
            Action::Fullscreen => "Full screen",
        }
    }
}

fn parse_key(name: &str) -> Option<Code> {
    let upper = name.to_ascii_uppercase();
    if let Some(digit) = upper.strip_prefix("NUMPAD") {
        return match digit {
            "0" => Some(Code::Numpad0),
            "1" => Some(Code::Numpad1),
            "2" => Some(Code::Numpad2),
            "3" => Some(Code::Numpad3),
            "4" => Some(Code::Numpad4),
            "5" => Some(Code::Numpad5),
            "6" => Some(Code::Numpad6),
            "7" => Some(Code::Numpad7),
            "8" => Some(Code::Numpad8),
            "9" => Some(Code::Numpad9),
            _ => None,
        };
    }
    if let Some(number) = upper.strip_prefix('F')
        && let Ok(n) = number.parse::<u8>()
        && (1..=24).contains(&n)
    {
        return Some(match n {
            1 => Code::F1,
            2 => Code::F2,
            3 => Code::F3,
            4 => Code::F4,
            5 => Code::F5,
            6 => Code::F6,
            7 => Code::F7,
            8 => Code::F8,
            9 => Code::F9,
            10 => Code::F10,
            11 => Code::F11,
            12 => Code::F12,
            13 => Code::F13,
            14 => Code::F14,
            15 => Code::F15,
            16 => Code::F16,
            17 => Code::F17,
            18 => Code::F18,
            19 => Code::F19,
            20 => Code::F20,
            21 => Code::F21,
            22 => Code::F22,
            23 => Code::F23,
            _ => Code::F24,
        });
    }
    if upper.len() == 1 {
        let c = upper.as_bytes()[0];
        if c.is_ascii_alphabetic() {
            const LETTERS: [Code; 26] = [
                Code::KeyA,
                Code::KeyB,
                Code::KeyC,
                Code::KeyD,
                Code::KeyE,
                Code::KeyF,
                Code::KeyG,
                Code::KeyH,
                Code::KeyI,
                Code::KeyJ,
                Code::KeyK,
                Code::KeyL,
                Code::KeyM,
                Code::KeyN,
                Code::KeyO,
                Code::KeyP,
                Code::KeyQ,
                Code::KeyR,
                Code::KeyS,
                Code::KeyT,
                Code::KeyU,
                Code::KeyV,
                Code::KeyW,
                Code::KeyX,
                Code::KeyY,
                Code::KeyZ,
            ];
            return Some(LETTERS[(c - b'A') as usize]);
        }
        if c.is_ascii_digit() {
            const DIGITS: [Code; 10] = [
                Code::Digit0,
                Code::Digit1,
                Code::Digit2,
                Code::Digit3,
                Code::Digit4,
                Code::Digit5,
                Code::Digit6,
                Code::Digit7,
                Code::Digit8,
                Code::Digit9,
            ];
            return Some(DIGITS[(c - b'0') as usize]);
        }
    }
    match upper.as_str() {
        "PRINTSCREEN" | "PRTSC" | "PRINT" | "SNAPSHOT" => Some(Code::PrintScreen),
        "INSERT" | "INS" => Some(Code::Insert),
        "DELETE" | "DEL" => Some(Code::Delete),
        "HOME" => Some(Code::Home),
        "END" => Some(Code::End),
        "PAGEUP" | "PGUP" => Some(Code::PageUp),
        "PAGEDOWN" | "PGDN" => Some(Code::PageDown),
        "SPACE" => Some(Code::Space),
        "ESCAPE" | "ESC" => Some(Code::Escape),
        "ENTER" | "RETURN" => Some(Code::Enter),
        "TAB" => Some(Code::Tab),
        "BACKSPACE" => Some(Code::Backspace),
        "UP" | "ARROWUP" => Some(Code::ArrowUp),
        "DOWN" | "ARROWDOWN" => Some(Code::ArrowDown),
        "LEFT" | "ARROWLEFT" => Some(Code::ArrowLeft),
        "RIGHT" | "ARROWRIGHT" => Some(Code::ArrowRight),
        "PAUSE" => Some(Code::Pause),
        "SCROLLLOCK" => Some(Code::ScrollLock),
        "MINUS" => Some(Code::Minus),
        "EQUAL" => Some(Code::Equal),
        "COMMA" => Some(Code::Comma),
        "PERIOD" => Some(Code::Period),
        "SLASH" => Some(Code::Slash),
        "BACKSLASH" => Some(Code::Backslash),
        "SEMICOLON" => Some(Code::Semicolon),
        "QUOTE" => Some(Code::Quote),
        "BACKQUOTE" => Some(Code::Backquote),
        "BRACKETLEFT" => Some(Code::BracketLeft),
        "BRACKETRIGHT" => Some(Code::BracketRight),
        _ => None,
    }
}

/// The name this module uses for a key egui reported.
///
/// egui has no way to express PrintScreen, Pause or ScrollLock, so those come
/// from the platform layer instead; see `record`.
pub fn name_of_egui_key(key: egui::Key) -> Option<&'static str> {
    use egui::Key as K;
    Some(match key {
        K::A => "A",
        K::B => "B",
        K::C => "C",
        K::D => "D",
        K::E => "E",
        K::F => "F",
        K::G => "G",
        K::H => "H",
        K::I => "I",
        K::J => "J",
        K::K => "K",
        K::L => "L",
        K::M => "M",
        K::N => "N",
        K::O => "O",
        K::P => "P",
        K::Q => "Q",
        K::R => "R",
        K::S => "S",
        K::T => "T",
        K::U => "U",
        K::V => "V",
        K::W => "W",
        K::X => "X",
        K::Y => "Y",
        K::Z => "Z",
        K::Num0 => "0",
        K::Num1 => "1",
        K::Num2 => "2",
        K::Num3 => "3",
        K::Num4 => "4",
        K::Num5 => "5",
        K::Num6 => "6",
        K::Num7 => "7",
        K::Num8 => "8",
        K::Num9 => "9",
        K::F1 => "F1",
        K::F2 => "F2",
        K::F3 => "F3",
        K::F4 => "F4",
        K::F5 => "F5",
        K::F6 => "F6",
        K::F7 => "F7",
        K::F8 => "F8",
        K::F9 => "F9",
        K::F10 => "F10",
        K::F11 => "F11",
        K::F12 => "F12",
        K::Insert => "Insert",
        K::Delete => "Delete",
        K::Home => "Home",
        K::End => "End",
        K::PageUp => "PageUp",
        K::PageDown => "PageDown",
        K::ArrowUp => "Up",
        K::ArrowDown => "Down",
        K::ArrowLeft => "Left",
        K::ArrowRight => "Right",
        K::Space => "Space",
        K::Enter => "Enter",
        K::Tab => "Tab",
        K::Backspace => "Backspace",
        K::Minus => "Minus",
        K::Equals => "Equal",
        K::Comma => "Comma",
        K::Period => "Period",
        K::Slash => "Slash",
        K::Backslash => "Backslash",
        K::Semicolon => "Semicolon",
        K::Quote => "Quote",
        K::Backtick => "Backquote",
        K::OpenBracket => "BracketLeft",
        K::CloseBracket => "BracketRight",
        _ => return None,
    })
}

/// Split a shortcut into its modifiers and its key.
pub fn split(shortcut: &str) -> (bool, bool, bool, bool, String) {
    let mut ctrl = false;
    let mut alt = false;
    let mut shift = false;
    let mut meta = false;
    let mut key = String::new();
    for part in shortcut.split('+') {
        let part = part.trim();
        match part.to_ascii_uppercase().as_str() {
            "" => {}
            "CTRL" | "CONTROL" => ctrl = true,
            "ALT" => alt = true,
            "SHIFT" => shift = true,
            "SUPER" | "WIN" | "META" | "CMD" => meta = true,
            _ => key = part.to_string(),
        }
    }
    (ctrl, alt, shift, meta, key)
}

pub fn join(ctrl: bool, alt: bool, shift: bool, meta: bool, key: &str) -> String {
    if key.is_empty() {
        return String::new();
    }
    let mut parts = Vec::new();
    if ctrl {
        parts.push("Ctrl");
    }
    if alt {
        parts.push("Alt");
    }
    if shift {
        parts.push("Shift");
    }
    if meta {
        parts.push("Super");
    }
    parts.push(key);
    parts.join("+")
}

/// What the settings screen shows for a shortcut.
pub fn describe(shortcut: &str) -> String {
    if shortcut.is_empty() {
        "Not set".to_string()
    } else {
        shortcut.to_string()
    }
}

pub fn parse(shortcut: &str) -> Result<HotKey, String> {
    let (ctrl, alt, shift, meta, key) = split(shortcut);
    if key.is_empty() {
        return Err("no key given".into());
    }
    let code = parse_key(&key).ok_or_else(|| format!("unknown key \"{key}\""))?;
    let mut modifiers = Modifiers::empty();
    if ctrl {
        modifiers |= Modifiers::CONTROL;
    }
    if alt {
        modifiers |= Modifiers::ALT;
    }
    if shift {
        modifiers |= Modifiers::SHIFT;
    }
    if meta {
        modifiers |= Modifiers::META;
    }
    Ok(HotKey::new(Some(modifiers), code))
}

pub fn is_valid(shortcut: &str) -> bool {
    parse(shortcut).is_ok()
}

/// Owns the registrations and turns incoming events back into actions.
pub struct Manager {
    inner: Option<GlobalHotKeyManager>,
    bound: HashMap<u32, Action>,
    registered: Vec<HotKey>,
    /// Shortcuts the system refused, with the reason. Shown in the settings.
    pub problems: Vec<String>,
}

impl Manager {
    pub fn new() -> Self {
        let (inner, problems) = match GlobalHotKeyManager::new() {
            Ok(manager) => (Some(manager), Vec::new()),
            Err(e) => (None, vec![format!("Shortcuts are unavailable: {e}")]),
        };
        Self {
            inner,
            bound: HashMap::new(),
            registered: Vec::new(),
            problems,
        }
    }

    /// Replace every registration. Called at startup and whenever the settings
    /// change, so there is one code path and no half applied state.
    pub fn apply(&mut self, keys: &Hotkeys) {
        let Some(manager) = &self.inner else {
            return;
        };
        for hotkey in self.registered.drain(..) {
            let _ = manager.unregister(hotkey);
        }
        self.bound.clear();
        self.problems.clear();

        for (action, shortcut) in [
            (Action::Region, &keys.region),
            (Action::Window, &keys.window),
            (Action::Fullscreen, &keys.fullscreen),
        ] {
            if shortcut.is_empty() {
                continue;
            }
            match parse(shortcut) {
                Ok(hotkey) => match manager.register(hotkey) {
                    Ok(()) => {
                        self.bound.insert(hotkey.id(), action);
                        self.registered.push(hotkey);
                    }
                    Err(e) => self.problems.push(format!(
                        "{} ({shortcut}) is taken by another program: {e}",
                        action.label()
                    )),
                },
                Err(e) => self
                    .problems
                    .push(format!("{} ({shortcut}): {e}", action.label())),
            }
        }
    }

    /// Drain everything the system has delivered since the last call.
    pub fn poll(&self) -> Vec<Action> {
        let mut actions = Vec::new();
        while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
            // Press and release both arrive; only the press should fire.
            if event.state != global_hotkey::HotKeyState::Pressed {
                continue;
            }
            if let Some(action) = self.bound.get(&event.id) {
                actions.push(*action);
            }
        }
        actions
    }

    /// Stop listening, so a shortcut being recorded is not swallowed by the
    /// registration that is about to be replaced.
    pub fn suspend(&mut self) {
        let Some(manager) = &self.inner else {
            return;
        };
        for hotkey in self.registered.drain(..) {
            let _ = manager.unregister(hotkey);
        }
        self.bound.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_key() {
        assert!(parse("PrintScreen").is_ok());
        assert!(parse("F12").is_ok());
    }

    #[test]
    fn modifiers_in_any_order_and_case() {
        let a = parse("Ctrl+Shift+S").unwrap();
        let b = parse("shift + CTRL + s").unwrap();
        assert_eq!(a.id(), b.id());
    }

    #[test]
    fn unknown_key_is_rejected() {
        assert!(parse("Ctrl+Frobnicate").is_err());
        assert!(parse("Ctrl").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn an_unbound_shortcut_reads_as_not_set() {
        assert_eq!(describe(""), "Not set");
        assert_eq!(describe("Alt+S"), "Alt+S");
    }

    #[test]
    fn modifiers_alone_do_not_make_a_shortcut() {
        assert_eq!(join(true, true, false, false, ""), "");
    }

    #[test]
    fn split_and_join_round_trip() {
        let (c, a, s, m, k) = split("Alt+PrintScreen");
        assert!(a && !c && !s && !m);
        assert_eq!(k, "PrintScreen");
        assert_eq!(join(c, a, s, m, &k), "Alt+PrintScreen");
    }

    #[test]
    fn every_key_egui_can_report_is_one_we_can_register() {
        for key in egui::Key::ALL {
            if let Some(name) = name_of_egui_key(*key) {
                assert!(parse_key(name).is_some(), "{name} should parse");
            }
        }
    }

    #[test]
    fn the_shipped_default_is_valid() {
        assert!(is_valid(&Hotkeys::default().region));
    }
}
