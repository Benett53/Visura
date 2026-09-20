//! Clipboard access.
//!
//! One connection is kept alive for the whole run. On X11 the clipboard is not
//! a place where data is stored but a promise from the process that owns the
//! selection, so dropping the handle after every copy would leave the user
//! with an empty clipboard.

use std::sync::{Mutex, OnceLock};

use crate::platform::Image;

fn clipboard() -> &'static Mutex<Option<arboard::Clipboard>> {
    static CLIPBOARD: OnceLock<Mutex<Option<arboard::Clipboard>>> = OnceLock::new();
    CLIPBOARD.get_or_init(|| Mutex::new(arboard::Clipboard::new().ok()))
}

fn with<T>(
    f: impl FnOnce(&mut arboard::Clipboard) -> Result<T, arboard::Error>,
) -> Result<T, String> {
    let mut guard = clipboard().lock().map_err(|_| "the clipboard is busy")?;
    if guard.is_none() {
        *guard = arboard::Clipboard::new().ok();
    }
    let handle = guard.as_mut().ok_or("the clipboard is unavailable")?;
    f(handle).map_err(|e| e.to_string())
}

pub fn copy_image(image: &Image) -> Result<(), String> {
    let data = arboard::ImageData {
        width: image.width as usize,
        height: image.height as usize,
        bytes: std::borrow::Cow::Borrowed(&image.rgba),
    };
    with(|c| c.set_image(data))
}

pub fn copy_text(text: &str) -> Result<(), String> {
    with(|c| c.set_text(text.to_string()))
}
