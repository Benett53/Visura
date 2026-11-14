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
//!
//! Window positions come from KWin as well. No interface hands them out, but
//! a script running inside KWin sees every window, so a few lines of
//! JavaScript are loaded, report once over D-Bus and are unloaded again.

use std::collections::HashMap;
use std::io::Read as _;
use std::os::fd::{AsFd as _, OwnedFd};
use std::sync::{Mutex, mpsc};
use std::time::Duration;

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
    capture("CaptureWorkspace", false)
}

/// The window that has the keyboard, with its decoration and without its
/// shadow. Rounded corners come back transparent.
pub fn capture_active_window() -> Result<Image, String> {
    capture("CaptureActiveWindow", true)
}

fn capture(method: &str, keep_alpha: bool) -> Result<Image, String> {
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
    options.insert("include-shadow", Value::Bool(false));

    let reply = connection.call_method(
        Some("org.kde.KWin"),
        "/org/kde/KWin/ScreenShot2",
        Some("org.kde.KWin.ScreenShot2"),
        method,
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

    to_rgba(&data, width, height, stride, format, keep_alpha)
}

/// Convert one QImage buffer into tightly packed RGBA. The desktop is
/// opaque whatever its alpha channel says; a single window keeps its alpha,
/// which is what makes its rounded corners transparent.
fn to_rgba(
    data: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    format: u32,
    keep_alpha: bool,
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
    // formats R, G, B, A.
    let bgra = match format {
        format::RGB32 | format::ARGB32 | format::ARGB32_PREMULTIPLIED => true,
        format::RGBX8888 | format::RGBA8888 | format::RGBA8888_PREMULTIPLIED => false,
        other => {
            return Err(format!(
                "KWin sent pixel format {other}, which is not supported"
            ));
        }
    };
    let has_alpha = keep_alpha && !matches!(format, format::RGB32 | format::RGBX8888);
    let premultiplied = matches!(
        format,
        format::ARGB32_PREMULTIPLIED | format::RGBA8888_PREMULTIPLIED
    );

    let mut rgba = Vec::with_capacity(row * height as usize);
    for line in data.chunks(stride).take(height as usize) {
        for px in line[..row].as_chunks::<4>().0 {
            let (r, g, b) = if bgra {
                (px[2], px[1], px[0])
            } else {
                (px[0], px[1], px[2])
            };
            let a = if has_alpha { px[3] } else { 255 };
            if has_alpha && premultiplied && a != 255 {
                rgba.extend_from_slice(&[
                    unpremultiply(r, a),
                    unpremultiply(g, a),
                    unpremultiply(b, a),
                    a,
                ]);
            } else {
                rgba.extend_from_slice(&[r, g, b, a]);
            }
        }
    }
    Ok(Image {
        width,
        height,
        rgba,
    })
}

fn unpremultiply(channel: u8, alpha: u8) -> u8 {
    if alpha == 0 {
        return 0;
    }
    ((channel as u32 * 255 + alpha as u32 / 2) / alpha as u32).min(255) as u8
}

// ---------------------------------------------------------------- windows ----

/// Runs inside KWin. The stacking order is bottom to top, so it is walked
/// backwards; what is left are the windows a person would call windows, plus
/// panels, on the current desktop and activity.
const WINDOW_SCRIPT: &str = r#"(function () {
    var out = [];
    var list = workspace.stackingOrder;
    var desktop = workspace.currentDesktop;
    var activity = workspace.currentActivity;
    for (var i = list.length - 1; i >= 0; i--) {
        var w = list[i];
        if (w.deleted || w.minimized || w.hidden) continue;
        if (!(w.normalWindow || w.dialog || w.utility || w.dock)) continue;
        if (!w.onAllDesktops && w.desktops.indexOf(desktop) < 0) continue;
        if (w.activities.length > 0 && w.activities.indexOf(activity) < 0) continue;
        var g = w.frameGeometry;
        out.push({
            x: g.x, y: g.y, width: g.width, height: g.height,
            title: w.caption, class: w.resourceClass, pid: w.pid, active: w.active
        });
    }
    var size = workspace.virtualScreenSize;
    callDBus("@SERVICE@", "@PATH@", "@INTERFACE@", "Report",
        JSON.stringify({ width: size.width, height: size.height, windows: out }));
})();
"#;

const REPORT_PATH: &str = "/io/github/visura/Windows";
const REPORT_INTERFACE: &str = "io.github.visura.Windows";

/// How long KWin gets to run the script and call back. It normally takes a
/// few milliseconds; the overlay must not wait long for outlines it can do
/// without.
const REPORT_TIMEOUT: Duration = Duration::from_millis(250);

/// The object KWin's script calls back on.
struct Receiver(Mutex<mpsc::Sender<String>>);

#[zbus::interface(name = "io.github.visura.Windows")]
impl Receiver {
    fn report(&self, json: &str) {
        if let Ok(sender) = self.0.lock() {
            let _ = sender.send(json.to_string());
        }
    }
}

/// The windows on screen as KWin sees them, front most first, as the JSON
/// the script above produces: logical coordinates plus the size of the
/// whole desktop. `None` when KWin does not answer.
pub fn window_report() -> Option<String> {
    let connection = Connection::session().ok()?;
    let service = connection.unique_name()?.to_string();
    let (sender, receiver) = mpsc::channel();
    connection
        .object_server()
        .at(REPORT_PATH, Receiver(Mutex::new(sender)))
        .ok()?;

    let pid = std::process::id();
    let plugin = format!("visura-windows-{pid}");
    let file = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|dir| !dir.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("{plugin}.js"));
    let script = WINDOW_SCRIPT
        .replace("@SERVICE@", &service)
        .replace("@PATH@", REPORT_PATH)
        .replace("@INTERFACE@", REPORT_INTERFACE);
    std::fs::write(&file, script).ok()?;

    let unload = || {
        let _ = connection.call_method(
            Some("org.kde.KWin"),
            "/Scripting",
            Some("org.kde.kwin.Scripting"),
            "unloadScript",
            &(plugin.as_str(),),
        );
    };

    let answer = (|| {
        let id: i32 = connection
            .call_method(
                Some("org.kde.KWin"),
                "/Scripting",
                Some("org.kde.kwin.Scripting"),
                "loadScript",
                &(file.to_string_lossy().as_ref(), plugin.as_str()),
            )
            .ok()?
            .body()
            .deserialize()
            .ok()?;
        if id < 0 {
            return None;
        }
        connection
            .call_method(
                Some("org.kde.KWin"),
                format!("/Scripting/Script{id}"),
                Some("org.kde.kwin.Script"),
                "run",
                &(),
            )
            .ok()?;
        receiver.recv_timeout(REPORT_TIMEOUT).ok()
    })();

    unload();
    let _ = std::fs::remove_file(&file);
    answer
}

/// Runs inside KWin: finds the X11 window with the given id and makes it the
/// active one. A script may do that without KWin's focus stealing prevention
/// getting a say, which is what keeps a freshly mapped overlay from opening
/// behind other windows.
const ACTIVATE_SCRIPT: &str = r#"(function () {
    var list = workspace.stackingOrder;
    for (var i = 0; i < list.length; i++) {
        if (list[i].windowId == @ID@) {
            workspace.activeWindow = list[i];
            return;
        }
    }
})();
"#;

/// Bring the X11 window `id` to the front and give it the focus. Returns
/// once the script has been started; KWin runs it in a few milliseconds.
pub fn activate_window(id: u32) {
    let Ok(connection) = Connection::session() else {
        return;
    };
    let plugin = format!("visura-activate-{}-{id}", std::process::id());
    let file = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|dir| !dir.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("{plugin}.js"));
    if std::fs::write(&file, ACTIVATE_SCRIPT.replace("@ID@", &id.to_string())).is_err() {
        return;
    }
    let call = |path: &str, interface: &str, method: &str| {
        connection.call_method(Some("org.kde.KWin"), path, Some(interface), method, &())
    };
    let loaded: Option<i32> = connection
        .call_method(
            Some("org.kde.KWin"),
            "/Scripting",
            Some("org.kde.kwin.Scripting"),
            "loadScript",
            &(file.to_string_lossy().as_ref(), plugin.as_str()),
        )
        .ok()
        .and_then(|reply| reply.body().deserialize().ok());
    if let Some(script) = loaded.filter(|id| *id >= 0) {
        let _ = call(
            &format!("/Scripting/Script{script}"),
            "org.kde.kwin.Script",
            "run",
        );
        // The script has no callback to wait for.
        std::thread::sleep(Duration::from_millis(150));
    }
    let _ = connection.call_method(
        Some("org.kde.KWin"),
        "/Scripting",
        Some("org.kde.kwin.Scripting"),
        "unloadScript",
        &(plugin.as_str(),),
    );
    let _ = std::fs::remove_file(&file);
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
        let image = to_rgba(&rgbx, 2, 1, 12, format::RGBX8888, false).unwrap();
        assert_eq!(image.rgba, [10, 20, 30, 255, 40, 50, 60, 255]);

        let bgra = [30, 20, 10, 255, 60, 50, 40, 255];
        let image = to_rgba(&bgra, 2, 1, 8, format::ARGB32, false).unwrap();
        assert_eq!(image.rgba, [10, 20, 30, 255, 40, 50, 60, 255]);
    }

    #[test]
    fn a_window_keeps_its_alpha_and_loses_the_premultiplication() {
        // Half transparent red and a fully transparent pixel, B G R A.
        let data = [0, 0, 128, 128, 0, 0, 0, 0];
        let image = to_rgba(&data, 2, 1, 8, format::ARGB32_PREMULTIPLIED, true).unwrap();
        assert_eq!(image.rgba, [255, 0, 0, 128, 0, 0, 0, 0]);
        // The same bytes for the desktop are simply opaque.
        let image = to_rgba(&data, 2, 1, 8, format::ARGB32_PREMULTIPLIED, false).unwrap();
        assert_eq!(image.rgba, [128, 0, 0, 255, 0, 0, 0, 255]);
        // A format without alpha stays opaque even for a window.
        let image = to_rgba(&data, 2, 1, 8, format::RGB32, true).unwrap();
        assert_eq!(image.rgba[3], 255);
    }

    #[test]
    fn short_buffers_and_unknown_formats_are_refused() {
        assert!(to_rgba(&[0; 7], 2, 1, 8, format::RGBA8888, false).is_err());
        assert!(to_rgba(&[0; 8], 2, 1, 8, 99, false).is_err());
    }

    /// Needs a running Plasma session: `cargo test -- --ignored kwin_live --nocapture`
    #[test]
    #[ignore]
    fn kwin_live_report_and_window_capture() {
        let start = std::time::Instant::now();
        let report = window_report().expect("KWin did not answer");
        println!("report after {:?}: {report}", start.elapsed());
        let start = std::time::Instant::now();
        let window = capture_active_window().expect("no active window capture");
        println!(
            "active window {}×{} after {:?}",
            window.width,
            window.height,
            start.elapsed()
        );
    }
}
