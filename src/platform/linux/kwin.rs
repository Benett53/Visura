//! Screen capture straight from KWin, the KDE Plasma compositor.
//!
//! `org.kde.KWin.ScreenShot2` is the interface Spectacle itself uses. It hands
//! back raw pixels through a pipe: no PNG to encode and decode, no helper
//! process to start. On a 3440×1440 desktop that is about 15 ms, against
//! about 550 ms for `spectacle -b -n -f`, which is the delay between pressing
//! the key and the overlay appearing.
//!
//! KWin may refuse the call to programs it does not trust; the desktop entry
//! declares the interface (`X-KDE-DBUS-Restricted-Interfaces`) for that case,
//! and the caller falls back to Spectacle when it still fails.

use std::collections::HashMap;
use std::io::Read as _;
use std::os::fd::{AsFd as _, OwnedFd};

use zbus::blocking::Connection;
use zbus::zvariant::{Fd, OwnedValue, Value};

use super::Image;

/// QImage pixel formats KWin may answer with, as their enum values.
mod format {
    pub const RGB32: u32 = 4;
    pub const ARGB32: u32 = 5;
    pub const ARGB32_PREMULTIPLIED: u32 = 6;
    pub const RGBX8888: u32 = 16;
    pub const RGBA8888: u32 = 17;
    pub const RGBA8888_PREMULTIPLIED: u32 = 18;
}

fn number(map: &HashMap<String, OwnedValue>, key: &str) -> Option<u32> {
    match &**map.get(key)? {
        Value::U32(v) => Some(*v),
        Value::I32(v) => u32::try_from(*v).ok(),
        _ => None,
    }
}

/// The whole desktop at its physical resolution.
pub fn capture_workspace() -> Result<Image, String> {
    let connection = Connection::session().map_err(|e| format!("D-Bus: {e}"))?;
    let (read_end, write_end) = pipe()?;

    // KWin answers first and writes afterwards, but nothing promises that, and
    // a full pipe would then block it forever. Reading on a thread of its own
    // is safe either way.
    let reader = std::thread::spawn(move || {
        let mut data = Vec::new();
        std::fs::File::from(read_end)
            .read_to_end(&mut data)
            .map(|_| data)
    });

    let mut options: HashMap<&str, Value> = HashMap::new();
    options.insert("native-resolution", Value::Bool(true));
    options.insert("include-cursor", Value::Bool(false));
    options.insert("include-decoration", Value::Bool(true));

    let reply = connection.call_method(
        Some("org.kde.KWin"),
        "/org/kde/KWin/ScreenShot2",
        Some("org.kde.KWin.ScreenShot2"),
        "CaptureWorkspace",
        &(options, Fd::from(write_end.as_fd())),
    );
    // Our copy of the write end must close, or the reader never sees the end.
    drop(write_end);
    let reply = reply.map_err(|e| format!("KWin: {e}"))?;
    let info: HashMap<String, OwnedValue> = reply
        .body()
        .deserialize()
        .map_err(|e| format!("KWin answered unexpectedly: {e}"))?;

    let data = reader
        .join()
        .map_err(|_| "reading the KWin frame failed".to_string())?
        .map_err(|e| format!("reading the KWin frame failed: {e}"))?;

    let width = number(&info, "width").ok_or("KWin sent no width")?;
    let height = number(&info, "height").ok_or("KWin sent no height")?;
    let stride = number(&info, "stride").ok_or("KWin sent no stride")? as usize;
    let format = number(&info, "format").ok_or("KWin sent no format")?;

    to_rgba(&data, width, height, stride, format)
}

/// Convert one QImage buffer into tightly packed, opaque RGBA.
fn to_rgba(
    data: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    format: u32,
) -> Result<Image, String> {
    let row = width as usize * 4;
    if width == 0
        || height == 0
        || stride < row
        || data.len() < stride * (height as usize - 1) + row
    {
        return Err(format!(
            "KWin sent {} bytes for {width}×{height} (stride {stride})",
            data.len()
        ));
    }
    // Little endian: the 32-bit formats are B, G, R, A in memory, the 8888
    // formats R, G, B, A. Premultiplied alpha does not matter, the desktop
    // is opaque.
    let bgra = match format {
        format::RGB32 | format::ARGB32 | format::ARGB32_PREMULTIPLIED => true,
        format::RGBX8888 | format::RGBA8888 | format::RGBA8888_PREMULTIPLIED => false,
        other => {
            return Err(format!(
                "KWin sent pixel format {other}, which is not supported"
            ));
        }
    };

    let mut rgba = Vec::with_capacity(row * height as usize);
    for line in data.chunks(stride).take(height as usize) {
        for px in line[..row].as_chunks::<4>().0 {
            if bgra {
                rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
            } else {
                rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
        }
    }
    Ok(Image {
        width,
        height,
        rgba,
    })
}

fn pipe() -> Result<(OwnedFd, OwnedFd), String> {
    use std::os::fd::FromRawFd as _;

    let mut fds = [0; 2];
    // SAFETY: `fds` has room for the two descriptors pipe2 writes.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(format!("pipe: {}", std::io::Error::last_os_error()));
    }
    // SAFETY: both descriptors were just created and are owned by nobody else.
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_byte_orders_and_padded_rows_convert() {
        // 2×1 pixels, rows padded to 12 bytes.
        let rgbx = [10, 20, 30, 0, 40, 50, 60, 0, 99, 99, 99, 99];
        let image = to_rgba(&rgbx, 2, 1, 12, format::RGBX8888).unwrap();
        assert_eq!(image.rgba, [10, 20, 30, 255, 40, 50, 60, 255]);

        let bgra = [30, 20, 10, 255, 60, 50, 40, 255];
        let image = to_rgba(&bgra, 2, 1, 8, format::ARGB32).unwrap();
        assert_eq!(image.rgba, [10, 20, 30, 255, 40, 50, 60, 255]);
    }

    #[test]
    fn short_buffers_and_unknown_formats_are_refused() {
        assert!(to_rgba(&[0; 7], 2, 1, 8, format::RGBA8888).is_err());
        assert!(to_rgba(&[0; 8], 2, 1, 8, 99).is_err());
    }
}
