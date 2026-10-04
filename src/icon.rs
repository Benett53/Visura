//! The application icon, embedded once and decoded on demand.

/// Decoded icon at the given edge length, as RGBA.
pub fn rgba_sized(edge: u32) -> Option<(Vec<u8>, u32, u32)> {
    let decoded = image::load_from_memory(include_bytes!("../assets/icon.png")).ok()?;
    // Lanczos keeps the thin corner marks crisp at tray and title bar sizes.
    let scaled = decoded
        .resize(edge, edge, image::imageops::FilterType::Lanczos3)
        .to_rgba8();
    let (w, h) = (scaled.width(), scaled.height());
    Some((scaled.into_raw(), w, h))
}

/// Tray sized icon. Windows scales anything else, usually badly.
#[cfg(windows)]
pub fn rgba() -> Option<(Vec<u8>, u32, u32)> {
    rgba_sized(32)
}

/// The logo as a texture for the interface, made once per run.
pub fn texture(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let id = egui::Id::new("visura-logo");
    if let Some(texture) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return Some(texture);
    }
    let (rgba, width, height) = rgba_sized(64)?;
    let texture = ctx.load_texture(
        "visura-logo",
        egui::ColorImage::from_rgba_unmultiplied([width as usize, height as usize], &rgba),
        egui::TextureOptions::LINEAR,
    );
    ctx.data_mut(|d| d.insert_temp(id, texture.clone()));
    Some(texture)
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
