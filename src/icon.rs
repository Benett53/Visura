//! The application icon, embedded once and decoded on demand.

/// Decoded icon at the given edge length, as RGBA.
pub fn rgba_sized(edge: u32) -> Option<(Vec<u8>, u32, u32)> {
    let decoded = image::load_from_memory(include_bytes!("../assets/icon.png")).ok()?;
    let scaled = decoded.thumbnail(edge, edge).to_rgba8();
    let (w, h) = (scaled.width(), scaled.height());
    Some((scaled.into_raw(), w, h))
}

/// Tray sized icon. Windows scales anything else, usually badly.
#[cfg(windows)]
pub fn rgba() -> Option<(Vec<u8>, u32, u32)> {
    rgba_sized(32)
}

/// Window and taskbar icon for the main window.
pub fn window_icon() -> Option<egui::IconData> {
    let (rgba, width, height) = rgba_sized(64)?;
    Some(egui::IconData {
        rgba,
        width,
        height,
    })
}
