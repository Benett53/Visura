//! Linux backend.
//!
//! X11 is the native path: it can read the whole screen and the geometry of
//! every window, which is what the selection overlay is built on.
//!
//! Under Wayland neither is allowed. XWayland answers the connection but
//! `GetImage` on the root window fails, because the root is never rendered.
//! The capture then falls back to the screenshot helper the desktop ships,
//! which yields the same frozen frame the overlay needs. Window outlines are
//! not available that way, so the overlay simply does not draw them.

use std::io::Write as _;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::SystemTime;

use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{
    AtomEnum, ConnectionExt, ImageFormat, Screen, Window, get_geometry, get_image, get_property,
    translate_coordinates,
};
use x11rb::rust_connection::RustConnection;

use super::{Image, LocalTime, Rect, WindowInfo};

// ------------------------------------------------------------- connection ----

struct Display {
    conn: RustConnection,
    root: Window,
    screen: Screen,
}

/// One connection for the whole process. Opening an X connection per capture
/// costs a round trip that is plainly visible when the overlay appears.
fn display() -> Option<&'static Display> {
    static DISPLAY: OnceLock<Option<Display>> = OnceLock::new();
    DISPLAY
        .get_or_init(|| {
            let (conn, screen_num) = x11rb::connect(None).ok()?;
            let screen = conn.setup().roots.get(screen_num)?.clone();
            Some(Display {
                root: screen.root,
                screen,
                conn,
            })
        })
        .as_ref()
}

// ---------------------------------------------------------------- screen ----

pub fn virtual_screen() -> Rect {
    match display() {
        // The root window already spans every monitor, so its geometry is the
        // virtual desktop without asking RandR.
        Some(d) => Rect::new(
            0,
            0,
            d.screen.width_in_pixels as i32,
            d.screen.height_in_pixels as i32,
        ),
        None => Rect::new(0, 0, 0, 0),
    }
}

/// The usable area of the monitor the mouse is on.
///
/// Used to put the window in the middle of the screen someone is actually
/// looking at, whatever its resolution.
pub fn work_area_at_cursor() -> Rect {
    let whole = virtual_screen();
    let Some(d) = display() else {
        return whole;
    };
    let (cx, cy) = cursor_position();
    let Some(reply) = d
        .conn
        .randr_get_monitors(d.root, true)
        .ok()
        .and_then(|c| c.reply().ok())
    else {
        return whole;
    };
    for monitor in &reply.monitors {
        let rect = Rect::new(
            monitor.x as i32,
            monitor.y as i32,
            monitor.width as i32,
            monitor.height as i32,
        );
        if rect.contains(cx, cy) {
            return rect;
        }
    }
    whole
}

/// Whether a Super key is held. egui has no modifier for it.
pub fn super_key_down() -> bool {
    let Some(d) = display() else {
        return false;
    };
    let Some(pressed) = d.conn.query_keymap().ok().and_then(|c| c.reply().ok()) else {
        return false;
    };
    // XK_Super_L and XK_Super_R
    [0xffebu32, 0xffec].iter().any(|sym| {
        keycode_for(d, *sym)
            .is_some_and(|code| pressed.keys[(code / 8) as usize] & (1 << (code % 8)) != 0)
    })
}

/// Keys egui cannot report, polled while a shortcut is being recorded.
pub fn poll_special_key() -> Option<&'static str> {
    let d = display()?;
    let pressed = d.conn.query_keymap().ok()?.reply().ok()?.keys;
    for (keysym, name) in [
        (0xff61u32, "PrintScreen"),
        (0xff13, "Pause"),
        (0xff14, "ScrollLock"),
    ] {
        let Some(code) = keycode_for(d, keysym) else {
            continue;
        };
        // query_keymap returns a bitmap of the 256 possible keycodes.
        if pressed[(code / 8) as usize] & (1 << (code % 8)) != 0 {
            return Some(name);
        }
    }
    None
}

fn keycode_for(d: &Display, keysym: u32) -> Option<u8> {
    let setup = d.conn.setup();
    let first = setup.min_keycode;
    let count = setup.max_keycode - setup.min_keycode + 1;
    let reply = d
        .conn
        .get_keyboard_mapping(first, count)
        .ok()?
        .reply()
        .ok()?;
    let per_code = reply.keysyms_per_keycode as usize;
    if per_code == 0 {
        return None;
    }
    reply
        .keysyms
        .chunks(per_code)
        .position(|syms| syms.contains(&keysym))
        .map(|i| first + i as u8)
}

/// X11 has no equivalent of a window placement, so the caller falls back to
/// setting size, position and visibility one command at a time.
pub struct Placement;

pub fn save_window_placement() -> Option<Placement> {
    None
}

pub fn restore_window_placement(_placement: &Placement) -> bool {
    false
}

/// X11 hides a window synchronously and has no open or close animation, so
/// the two helpers the Windows backend needs for that have no counterpart.
///
/// This always reports the window as gone, which is what the capture waits for.
pub fn own_window_is_visible() -> bool {
    false
}

/// Where the mouse is right now, in physical screen pixels.
pub fn cursor_position() -> (i32, i32) {
    let Some(d) = display() else {
        return (0, 0);
    };
    match d
        .conn
        .query_pointer(d.root)
        .ok()
        .and_then(|c| c.reply().ok())
    {
        Some(p) => (p.root_x as i32, p.root_y as i32),
        None => (0, 0),
    }
}

/// Read a rectangle of the root window.
///
/// The read is split into horizontal strips. A full 4K desktop in one GetImage
/// reply is around 30 MB, which some X servers refuse outright.
pub fn capture_rect(r: Rect) -> Result<Image, String> {
    if r.is_empty() {
        return Err("empty capture rectangle".into());
    }
    let d = display().ok_or("no X11 display; Visura needs X11 or XWayland")?;

    const STRIP: i32 = 128;
    let mut rgba = Vec::with_capacity((r.w as usize) * (r.h as usize) * 4);

    let mut y = 0;
    while y < r.h {
        let rows = STRIP.min(r.h - y);
        let cookie = get_image(
            &d.conn,
            ImageFormat::Z_PIXMAP,
            d.root,
            r.x as i16,
            (r.y + y) as i16,
            r.w as u16,
            rows as u16,
            !0,
        )
        .map_err(|e| format!("GetImage failed: {e}"))?;
        let reply = cookie
            .reply()
            .map_err(|e| format!("GetImage failed: {e}"))?;

        // 24 and 32 bit TrueColor visuals both hand back four bytes per pixel
        // in BGRX order on little endian machines, which is every machine that
        // matters here.
        if reply.depth != 24 && reply.depth != 32 {
            return Err(format!("unsupported colour depth {}", reply.depth));
        }
        for px in reply.data.chunks_exact(4) {
            rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
        }
        y += rows;
    }

    Ok(Image {
        width: r.w as u32,
        height: r.h as u32,
        rgba,
    })
}

/// The whole desktop, as one image plus the rectangle it covers.
///
/// X11 first. Under Wayland that fails and the desktop helper takes over; the
/// size then comes from the image the helper produced, because the X server
/// does not know the real layout in that case.
pub fn capture_screen() -> Result<(Image, Rect), String> {
    let screen = virtual_screen();
    let x11_error = if screen.is_empty() {
        "no X11 screen".to_string()
    } else {
        match capture_rect(screen) {
            Ok(image) => return Ok((image, screen)),
            Err(e) => e,
        }
    };

    match capture_via_helper() {
        Ok(image) => {
            let rect = Rect::new(0, 0, image.width as i32, image.height as i32);
            Ok((image, rect))
        }
        Err(helper_error) => Err(format!(
            "the screen could not be read.\nX11: {x11_error}\n{helper_error}"
        )),
    }
}

/// Ask whatever screenshot helper the desktop ships for a full screen PNG.
///
/// Every Wayland compositor keeps screen contents to itself, and the portal
/// that exists for this opens the compositor's own region picker, which would
/// replace the overlay rather than feed it. These helpers hand back a plain
/// image instead, which is exactly what is needed.
fn capture_via_helper() -> Result<Image, String> {
    let target = std::env::temp_dir().join(format!("visura-frame-{}.png", std::process::id()));
    let path = target.to_string_lossy().into_owned();

    let candidates: [(&str, Vec<&str>); 5] = [
        // wlroots: Sway, Hyprland, river
        ("grim", vec![&path]),
        ("wayshot", vec!["-f", &path]),
        // KDE
        ("spectacle", vec!["-b", "-n", "-f", "-o", &path]),
        // GNOME
        ("gnome-screenshot", vec!["-f", &path]),
        // X11, in case the connection failed for some other reason
        ("scrot", vec!["-o", &path]),
    ];

    let mut tried = Vec::new();
    for (program, args) in candidates {
        let _ = std::fs::remove_file(&target);
        let Ok(status) = std::process::Command::new(program).args(&args).status() else {
            continue; // not installed
        };
        tried.push(program);
        if !status.success() {
            continue;
        }
        let Ok(bytes) = std::fs::read(&target) else {
            continue;
        };
        let _ = std::fs::remove_file(&target);
        let decoded = image::load_from_memory(&bytes)
            .map_err(|e| format!("{program} produced an unreadable image: {e}"))?
            .to_rgba8();
        let (width, height) = (decoded.width(), decoded.height());
        return Ok(Image {
            width,
            height,
            rgba: decoded.into_raw(),
        });
    }

    if tried.is_empty() {
        Err("no screenshot helper found. Install grim (wlroots), spectacle (KDE) or gnome-screenshot (GNOME)".into())
    } else {
        Err(format!("the helper failed: tried {}", tried.join(", ")))
    }
}

// --------------------------------------------------------------- windows ----

fn atom(d: &Display, name: &str) -> Option<u32> {
    d.conn
        .intern_atom(false, name.as_bytes())
        .ok()?
        .reply()
        .ok()
        .map(|r| r.atom)
}

fn text_property(d: &Display, window: Window, property: u32) -> Option<String> {
    let reply = get_property(&d.conn, false, window, property, AtomEnum::ANY, 0, 1024)
        .ok()?
        .reply()
        .ok()?;
    if reply.value.is_empty() {
        return None;
    }
    Some(String::from_utf8_lossy(&reply.value).into_owned())
}

fn cardinals(d: &Display, window: Window, property: u32) -> Option<Vec<u32>> {
    let reply = get_property(&d.conn, false, window, property, AtomEnum::ANY, 0, 64)
        .ok()?
        .reply()
        .ok()?;
    Some(reply.value32()?.collect())
}

/// Top level windows, front most first.
///
/// `_NET_CLIENT_LIST_STACKING` is the window manager's own stacking order,
/// bottom to top, so it is reversed here. Without a compliant window manager
/// the list comes back empty and the overlay simply shows no highlight.
pub fn windows_in_z_order() -> Vec<WindowInfo> {
    let Some(d) = display() else {
        return Vec::new();
    };
    let Some(stacking) = atom(d, "_NET_CLIENT_LIST_STACKING") else {
        return Vec::new();
    };
    let Some(list) = cardinals(d, d.root, stacking) else {
        return Vec::new();
    };

    let net_name = atom(d, "_NET_WM_NAME");
    let frame_extents = atom(d, "_NET_FRAME_EXTENTS");
    let net_pid = atom(d, "_NET_WM_PID");
    let screen = virtual_screen();

    let mut windows = Vec::new();
    for &window in list.iter().rev() {
        if let Some(info) = describe_window(d, window, net_name, frame_extents, net_pid, &screen) {
            windows.push(info);
        }
    }
    windows
}

fn describe_window(
    d: &Display,
    window: Window,
    net_name: Option<u32>,
    frame_extents: Option<u32>,
    net_pid: Option<u32>,
    screen: &Rect,
) -> Option<WindowInfo> {
    let geometry = get_geometry(&d.conn, window).ok()?.reply().ok()?;
    let origin = translate_coordinates(&d.conn, window, d.root, 0, 0)
        .ok()?
        .reply()
        .ok()?;

    // Reparenting window managers put the decoration outside the client area.
    // _NET_FRAME_EXTENTS gives left, right, top, bottom of that decoration.
    let extents = frame_extents
        .and_then(|a| cardinals(d, window, a))
        .filter(|v| v.len() >= 4)
        .unwrap_or_else(|| vec![0, 0, 0, 0]);

    let rect = Rect::new(
        origin.dst_x as i32 - extents[0] as i32,
        origin.dst_y as i32 - extents[2] as i32,
        geometry.width as i32 + extents[0] as i32 + extents[1] as i32,
        geometry.height as i32 + extents[2] as i32 + extents[3] as i32,
    )
    .intersect(screen);
    if rect.w < 8 || rect.h < 8 {
        return None;
    }

    let title = net_name
        .and_then(|a| text_property(d, window, a))
        .or_else(|| text_property(d, window, AtomEnum::WM_NAME.into()))
        .unwrap_or_default();
    if title.is_empty() {
        return None;
    }

    let app = net_pid
        .and_then(|a| cardinals(d, window, a))
        .and_then(|v| v.first().copied())
        .and_then(|pid| std::fs::read_to_string(format!("/proc/{pid}/comm")).ok())
        .map(|s| s.trim().to_string())
        .or_else(|| {
            // WM_CLASS is two null separated strings; the second is the class.
            text_property(d, window, AtomEnum::WM_CLASS.into())
                .and_then(|s| s.split(char::from(0)).nth(1).map(str::to_string))
        })
        .unwrap_or_default();

    Some(WindowInfo { rect, title, app })
}

pub fn foreground_window() -> Option<WindowInfo> {
    let d = display()?;
    let active = atom(d, "_NET_ACTIVE_WINDOW")?;
    let window = cardinals(d, d.root, active)?.first().copied()?;
    describe_window(
        d,
        window,
        atom(d, "_NET_WM_NAME"),
        atom(d, "_NET_FRAME_EXTENTS"),
        atom(d, "_NET_WM_PID"),
        &virtual_screen(),
    )
}

// ------------------------------------------------------------------ time ----

fn to_local(epoch_secs: i64, millis: u32) -> LocalTime {
    unsafe {
        let t = epoch_secs as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return LocalTime {
                year: 1970,
                month: 1,
                day: 1,
                hour: 0,
                minute: 0,
                second: 0,
                millis: 0,
            };
        }
        LocalTime {
            year: tm.tm_year + 1900,
            month: tm.tm_mon as u32 + 1,
            day: tm.tm_mday as u32,
            hour: tm.tm_hour as u32,
            minute: tm.tm_min as u32,
            second: tm.tm_sec as u32,
            millis,
        }
    }
}

pub fn local_now() -> LocalTime {
    local_from_system_time(SystemTime::now())
}

pub fn local_from_system_time(t: SystemTime) -> LocalTime {
    let d = t.duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
    to_local(d.as_secs() as i64, d.subsec_millis())
}

// ------------------------------------------------------------ drag source ----

/// X11 drag and drop (XDND) needs the drag source to own a selection, grab the
/// pointer and answer messages from the window under the cursor for the whole
/// drag. winit owns the event loop and does not expose enough of it to do that
/// correctly, so this puts the file on the clipboard instead and the caller
/// tells the user. Tracked as a known limitation.
pub fn start_file_drag(paths: &[PathBuf]) -> Result<(), String> {
    let uris: Vec<String> = paths
        .iter()
        .map(|p| format!("file://{}", p.display()))
        .collect();
    if uris.is_empty() {
        return Err("nothing to drag".into());
    }
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    clipboard
        .set_text(uris.join("\n"))
        .map_err(|e| e.to_string())?;
    Err("drag out is not available on X11; the file path was copied instead".into())
}

// ----------------------------------------------------------------- shell ----

fn run(program: &str, args: &[&str]) -> Option<std::process::Output> {
    std::process::Command::new(program).args(args).output().ok()
}

pub fn reveal_in_file_manager(path: &Path) {
    // The freedesktop interface for "show this file" is a D-Bus call that not
    // every file manager implements, so the folder is opened instead.
    if let Some(parent) = path.parent() {
        let _ = std::process::Command::new("xdg-open").arg(parent).spawn();
    }
}

pub fn open_path(path: &Path) {
    let _ = std::process::Command::new("xdg-open").arg(path).spawn();
}

/// Move files into the freedesktop trash so the history delete stays undoable.
pub fn move_to_trash(paths: &[PathBuf]) -> Result<(), String> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or("HOME is not set")?;
    let trash = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"))
        .join("Trash");
    let files = trash.join("files");
    let info = trash.join("info");
    std::fs::create_dir_all(&files).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&info).map_err(|e| e.to_string())?;

    for path in paths {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .ok_or("path has no file name")?;

        // Never overwrite something already in the trash.
        let mut target = files.join(&name);
        let mut suffix = 1;
        while target.exists() {
            target = files.join(format!("{suffix}_{name}"));
            suffix += 1;
        }
        let stamp = local_now();
        let record = format!(
            "[Trash Info]\nPath={}\nDeletionDate={:04}-{:02}-{:02}T{:02}:{:02}:{:02}\n",
            path.display(),
            stamp.year,
            stamp.month,
            stamp.day,
            stamp.hour,
            stamp.minute,
            stamp.second
        );
        let info_name = target.file_name().unwrap().to_string_lossy().into_owned();
        std::fs::write(info.join(format!("{info_name}.trashinfo")), record)
            .map_err(|e| e.to_string())?;
        std::fs::rename(path, &target).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn pick_folder(start: &Path) -> Option<PathBuf> {
    let start = start.to_string_lossy().into_owned();
    let candidates: [(&str, Vec<&str>); 2] = [
        (
            "zenity",
            vec!["--file-selection", "--directory", "--filename", &start],
        ),
        ("kdialog", vec!["--getexistingdirectory", &start]),
    ];
    for (program, args) in candidates {
        if let Some(output) = run(program, &args)
            && output.status.success()
        {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
    }
    None
}

fn xdg_user_dir(key: &str, fallback: &str) -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    if let Some(dir) = std::env::var_os(key) {
        return PathBuf::from(dir);
    }
    // user-dirs.dirs is the file xdg-user-dirs writes; parsing it beats
    // guessing for localised folder names.
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    if let Ok(text) = std::fs::read_to_string(config.join("user-dirs.dirs")) {
        for line in text.lines() {
            if let Some(value) = line.strip_prefix(&format!("{key}=")) {
                let value = value.trim().trim_matches('"');
                if let Some(rest) = value.strip_prefix("$HOME/") {
                    return home.join(rest);
                }
                return PathBuf::from(value);
            }
        }
    }
    home.join(fallback)
}

pub fn pictures_dir() -> PathBuf {
    xdg_user_dir("XDG_PICTURES_DIR", "Pictures")
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".config")
        })
        .join("visura")
}

pub fn cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".cache")
        })
        .join("visura")
}

// ------------------------------------------------------------- autostart ----

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let dir = config_dir()
        .parent()
        .map(|p| p.join("autostart"))
        .ok_or("no config directory")?;
    let file = dir.join("visura.desktop");
    if !enabled {
        let _ = std::fs::remove_file(&file);
        return Ok(());
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let entry = format!(
        "[Desktop Entry]\nType=Application\nName=Visura\nExec={} --background\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
        exe.display()
    );
    std::fs::write(&file, entry).map_err(|e| e.to_string())
}

// -------------------------------------------------------- single instance ----

fn socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("visura.sock")
}

pub struct InstanceGuard(UnixListener);

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(socket_path());
    }
}

/// Returns `None` when another copy already runs; that copy is asked to show
/// its window instead.
pub fn acquire_single_instance() -> Option<InstanceGuard> {
    let path = socket_path();
    match UnixListener::bind(&path) {
        Ok(listener) => Some(InstanceGuard(listener)),
        Err(_) => {
            // Either a live instance or a socket left behind by a crash.
            match UnixStream::connect(&path) {
                Ok(mut stream) => {
                    let _ = stream.write_all(b"show");
                    None
                }
                Err(_) => {
                    let _ = std::fs::remove_file(&path);
                    UnixListener::bind(&path).ok().map(InstanceGuard)
                }
            }
        }
    }
}

/// Call `on_show` whenever another copy of the program is started and asks
/// this one to come to the front. The guard is kept alive by the thread.
pub fn spawn_show_listener(guard: InstanceGuard, on_show: impl Fn() + Send + 'static) {
    std::thread::Builder::new()
        .name("visura-instance".into())
        .spawn(move || {
            let guard = guard;
            for stream in guard.0.incoming() {
                if stream.is_ok() {
                    on_show();
                }
            }
        })
        .ok();
}
