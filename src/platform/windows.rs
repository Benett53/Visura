//! Windows backend: GDI screen capture, window enumeration through the window
//! manager's own z-order, OLE drag and drop, and the small shell helpers.

use std::ffi::OsString;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::time::SystemTime;

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, FILETIME, HANDLE, HWND, LPARAM, LRESULT, MAX_PATH, POINT,
    RECT, WAIT_OBJECT_0, WPARAM,
};
use windows::Win32::Graphics::Dwm::{
    DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DWMWA_TRANSITIONS_FORCEDISABLED,
    DwmGetWindowAttribute, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap,
    CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, GetMonitorInfoW,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint, ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree, IDataObject,
};
use windows::Win32::System::Ole::{
    DROPEFFECT, DROPEFFECT_COPY, DoDragDrop, IDropSource, IDropSource_Impl, OleInitialize,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegDeleteValueW, RegOpenKeyExW,
    RegSetValueExW,
};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::System::Threading::{
    CreateEventW, CreateMutexW, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW, SetEvent, WaitForMultipleObjects,
};
use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_PAUSE, VK_SCROLL, VK_SNAPSHOT,
};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    BHID_DataObject, DefSubclassProc, FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_SILENT,
    FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, IShellItem, IShellItemArray,
    SHCreateItemFromParsingName, SHCreateShellItemArrayFromIDLists, SHFILEOPSTRUCTW,
    SHFileOperationW, SHParseDisplayName, SIGDN_FILESYSPATH, SetWindowSubclass, ShellExecuteW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GW_HWNDPREV, GWL_EXSTYLE, GetCursorPos, GetForegroundWindow,
    GetSystemMetrics, GetWindow, GetWindowLongW, GetWindowPlacement, GetWindowRect,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, HTCLIENT, IsIconic, IsWindow,
    IsWindowVisible, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SW_HIDE, SW_SHOWMAXIMIZED, SW_SHOWMINIMIZED, SW_SHOWNOACTIVATE, SW_SHOWNORMAL, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, SetForegroundWindow, SetWindowPlacement, SetWindowPos, WINDOWPLACEMENT,
    WM_NCHITTEST, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
};
use windows::core::{BOOL, HRESULT, PCWSTR, PWSTR, implement};

use super::{Image, LocalTime, Rect, Request, WindowInfo};

fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
}

// ---------------------------------------------------------------- screen ----

pub fn virtual_screen() -> Rect {
    unsafe {
        Rect::new(
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

/// Where the mouse is right now, in physical screen pixels.
pub fn cursor_position() -> (i32, i32) {
    unsafe {
        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        (p.x, p.y)
    }
}

/// Copy a rectangle of the desktop into an RGBA buffer.
///
/// `CAPTUREBLT` is what makes layered windows and most overlays show up; it is
/// the same flag ShareX uses. Without it the shot silently loses content.
pub fn capture_rect(r: Rect) -> Result<Image, String> {
    if r.is_empty() {
        return Err("empty capture rectangle".into());
    }
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return Err("no screen device context".into());
        }
        let mem = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, r.w, r.h);
        let previous = SelectObject(mem, bitmap.into());

        let blit = BitBlt(
            mem,
            0,
            0,
            r.w,
            r.h,
            Some(screen),
            r.x,
            r.y,
            SRCCOPY | CAPTUREBLT,
        );

        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: r.w,
                // Negative height asks GDI for a top-down buffer, which saves
                // flipping every row afterwards.
                biHeight: -r.h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };

        let mut buffer = vec![0u8; (r.w as usize) * (r.h as usize) * 4];
        let copied = GetDIBits(
            mem,
            bitmap,
            0,
            r.h as u32,
            Some(buffer.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        );

        SelectObject(mem, previous);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);

        blit.map_err(|e| format!("BitBlt failed: {e}"))?;
        if copied == 0 {
            return Err("GetDIBits returned no scan lines".into());
        }

        // GDI hands back BGRA with an alpha byte that is not meaningful here.
        for px in buffer.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }

        Ok(Image {
            width: r.w as u32,
            height: r.h as u32,
            rgba: buffer,
        })
    }
}

/// The whole desktop, as one image plus the rectangle it covers.
pub fn capture_screen() -> Result<(Image, Rect), String> {
    let screen = virtual_screen();
    if screen.is_empty() {
        return Err("the screen size could not be determined".into());
    }
    Ok((capture_rect(screen)?, screen))
}

/// The usable area of the monitor the mouse is on, without the taskbar.
///
/// Used to put the window in the middle of the screen someone is actually
/// looking at, whatever its resolution.
/// The work area of the primary monitor: the one with the task bar's start
/// button, which always contains the desktop origin.
pub fn primary_work_area() -> Rect {
    unsafe {
        use windows::Win32::Graphics::Gdi::MONITOR_DEFAULTTOPRIMARY;
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info).as_bool() {
            let r = info.rcWork;
            Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top)
        } else {
            virtual_screen()
        }
    }
}

pub fn work_area_at_cursor() -> Rect {
    unsafe {
        let (x, y) = cursor_position();
        let monitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info).as_bool() {
            let r = info.rcWork;
            Rect::new(r.left, r.top, r.right - r.left, r.bottom - r.top)
        } else {
            virtual_screen()
        }
    }
}

/// The primary and secondary mouse button, as the user has them set up: with
/// the buttons swapped for left handed use, primary is the physical right one.
///
/// The overlay reads the buttons here rather than through egui. egui only
/// passes a click on once it knows where the pointer is, and it learns that
/// from a mouse movement; an overlay that opens under a mouse that has not
/// moved would otherwise swallow the click and look frozen.
pub fn mouse_buttons() -> (bool, bool) {
    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::{VK_LBUTTON, VK_RBUTTON};
        use windows::Win32::UI::WindowsAndMessaging::SM_SWAPBUTTON;
        // GetAsyncKeyState reports the physical buttons.
        let left = GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000 != 0;
        let right = GetAsyncKeyState(VK_RBUTTON.0 as i32) as u16 & 0x8000 != 0;
        if GetSystemMetrics(SM_SWAPBUTTON) != 0 {
            (right, left)
        } else {
            (left, right)
        }
    }
}

/// Whether a Ctrl key is held, read the same way as the mouse buttons.
pub fn ctrl_key_down() -> bool {
    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::VK_CONTROL;
        GetAsyncKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000 != 0
    }
}

/// Whether a Windows key is held. egui has no modifier for it.
pub fn super_key_down() -> bool {
    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::{VK_LWIN, VK_RWIN};
        [VK_LWIN, VK_RWIN]
            .iter()
            .any(|vk| GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000 != 0)
    }
}

/// Keys egui cannot report, polled while a shortcut is being recorded.
pub fn poll_special_key() -> Option<&'static str> {
    unsafe {
        for (vk, name) in [
            (VK_SNAPSHOT, "PrintScreen"),
            (VK_PAUSE, "Pause"),
            (VK_SCROLL, "ScrollLock"),
        ] {
            // The high bit means the key is down right now.
            if GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000 != 0 {
                return Some(name);
            }
        }
        None
    }
}

// --------------------------------------------------------------- windows ----

struct Collector {
    windows: Vec<WindowInfo>,
    screen: Rect,
    own_pid: u32,
}

/// Top level windows, front most first.
///
/// `EnumWindows` already walks the z-order, so the first hit under the cursor
/// is the window the user actually sees. That is the whole trick behind the
/// window highlight in the overlay.
pub fn windows_in_z_order() -> Vec<WindowInfo> {
    let mut collector = Collector {
        windows: Vec::new(),
        screen: virtual_screen(),
        own_pid: std::process::id(),
    };
    unsafe {
        let _ = EnumWindows(
            Some(enum_proc),
            LPARAM(&mut collector as *mut Collector as isize),
        );
    }
    collector.windows
}

unsafe extern "system" fn enum_proc(hwnd: HWND, param: LPARAM) -> BOOL {
    let collector = unsafe { &mut *(param.0 as *mut Collector) };
    if let Some(info) = unsafe { describe_window(hwnd, collector) } {
        collector.windows.push(info);
    }
    true.into()
}

unsafe fn describe_window(hwnd: HWND, collector: &Collector) -> Option<WindowInfo> {
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
            return None;
        }

        // Tool windows are palettes and helpers, never a capture target.
        let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
        if ex_style & WS_EX_TOOLWINDOW.0 != 0 {
            return None;
        }

        // Store apps keep hidden windows around; they are "cloaked", not
        // invisible, so IsWindowVisible alone is not enough.
        let mut cloaked = 0u32;
        if DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            (&raw mut cloaked).cast(),
            size_of::<u32>() as u32,
        )
        .is_ok()
            && cloaked != 0
        {
            return None;
        }

        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == collector.own_pid {
            return None;
        }

        let title = window_title(hwnd);
        if title.is_empty() {
            return None;
        }

        // The window rect includes the invisible resize border and the drop
        // shadow. The DWM frame bounds are what the user perceives as the
        // window, so the highlight lines up with what is on screen.
        let mut bounds = RECT::default();
        let have_bounds = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&raw mut bounds).cast(),
            size_of::<RECT>() as u32,
        )
        .is_ok();
        if !have_bounds {
            GetWindowRect(hwnd, &mut bounds).ok()?;
        }

        let rect = Rect::new(
            bounds.left,
            bounds.top,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
        )
        .intersect(&collector.screen);
        if rect.w < 8 || rect.h < 8 {
            return None;
        }

        Some(WindowInfo {
            rect,
            title,
            app: process_name(pid).unwrap_or_default(),
            areas: child_areas(hwnd, rect),
        })
    }
}

/// Panes smaller than this are skipped. Below it the outline would jump onto
/// every button and scroll bar, which is not what anyone hovering a window
/// wants to capture.
const MIN_AREA_WIDTH: i32 = 120;
const MIN_AREA_HEIGHT: i32 = 80;
/// A child reaching this close to every edge of its window is a container for
/// the whole window, not a pane, and would only duplicate the outline.
const FULL_COVER_MARGIN: i32 = 16;
/// Enough for any real layout; stops a pathological window with thousands of
/// children from slowing the overlay down.
const MAX_AREAS: usize = 64;

/// The child windows of a top level window that are worth offering as a
/// capture target on their own.
///
/// Chromium based programs (Chrome, Edge, Brave, and every Electron app such
/// as Discord or VS Code) put the web content into its own child window, which
/// is what lets a page be taken without the browser around it. Programs that
/// draw everything into one window simply have no such children, and the
/// whole window is used.
fn child_areas(hwnd: HWND, parent: Rect) -> Vec<Rect> {
    struct Collect {
        areas: Vec<Rect>,
        parent: Rect,
    }

    unsafe extern "system" fn visit(child: HWND, param: LPARAM) -> BOOL {
        let collect = unsafe { &mut *(param.0 as *mut Collect) };
        if collect.areas.len() >= MAX_AREAS {
            return false.into();
        }
        // Also false when any ancestor is hidden, which is what is wanted.
        if !unsafe { IsWindowVisible(child) }.as_bool() {
            return true.into();
        }
        let mut bounds = RECT::default();
        if unsafe { GetWindowRect(child, &mut bounds) }.is_err() {
            return true.into();
        }
        let parent = collect.parent;
        let rect = Rect::new(
            bounds.left,
            bounds.top,
            bounds.right - bounds.left,
            bounds.bottom - bounds.top,
        )
        .intersect(&parent);

        let big_enough = rect.w >= MIN_AREA_WIDTH && rect.h >= MIN_AREA_HEIGHT;
        let covers_everything = (rect.x - parent.x).abs() <= FULL_COVER_MARGIN
            && (rect.y - parent.y).abs() <= FULL_COVER_MARGIN
            && (rect.right() - parent.right()).abs() <= FULL_COVER_MARGIN
            && (rect.bottom() - parent.bottom()).abs() <= FULL_COVER_MARGIN;
        if big_enough && !covers_everything && !collect.areas.contains(&rect) {
            collect.areas.push(rect);
        }
        true.into()
    }

    let mut collect = Collect {
        areas: Vec::new(),
        parent,
    };
    unsafe {
        let _ = EnumChildWindows(
            Some(hwnd),
            Some(visit),
            LPARAM(&mut collect as *mut Collect as isize),
        );
    }
    collect.areas
}

fn window_title(hwnd: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        let written = GetWindowTextW(hwnd, &mut buf);
        OsString::from_wide(&buf[..written.max(0) as usize])
            .to_string_lossy()
            .into_owned()
    }
}

fn process_name(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; MAX_PATH as usize];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(
            handle,
            Default::default(),
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(handle);
        ok.ok()?;
        let path = PathBuf::from(OsString::from_wide(&buf[..len as usize]));
        Some(path.file_stem()?.to_string_lossy().into_owned())
    }
}

/// The main window, handed over by the windowing layer at startup.
///
/// Searching for it by process id was not good enough: a process has several
/// top level windows, and the message only windows that the tray icon and the
/// hotkey layer create look just like the real one from the outside.
static MAIN_WINDOW: AtomicIsize = AtomicIsize::new(0);

pub fn set_main_window(hwnd: isize) {
    MAIN_WINDOW.store(hwnd, Ordering::Relaxed);
    disable_window_animations();
    unsafe {
        let _ = SetWindowSubclass(
            HWND(hwnd as *mut std::ffi::c_void),
            Some(overlay_hit_test),
            OVERLAY_SUBCLASS_ID,
            0,
        );
    }
}

/// Set while the main window is the capture overlay.
static OVERLAY_ACTIVE: AtomicBool = AtomicBool::new(false);
const OVERLAY_SUBCLASS_ID: usize = 0x5649_5355;

/// Only a Linux question; Windows has one desktop protocol.
pub fn is_wayland() -> bool {
    false
}

/// The overlay covers the virtual desktop by position and size here, which
/// spans every monitor; full screen would cover only one.
pub fn overlay_fullscreen() -> bool {
    false
}

/// Windows captures the active window from the full frame; see Linux.
pub fn capture_active_window() -> Option<Result<Image, String>> {
    None
}

/// Tell the window whether it is the overlay right now.
///
/// While it is, every point of it is reported to Windows as ordinary client
/// area. Otherwise Windows still treats the outermost pixels as a frame: a
/// drag starting at the very edge of the screen moved or resized the overlay
/// instead of selecting, and the program stopped reacting until it was killed.
pub fn set_overlay_active(active: bool) {
    OVERLAY_ACTIVE.store(active, Ordering::Relaxed);
}

unsafe extern "system" fn overlay_hit_test(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if message == WM_NCHITTEST && OVERLAY_ACTIVE.load(Ordering::Relaxed) {
        return LRESULT(HTCLIENT as isize);
    }
    unsafe { DefSubclassProc(hwnd, message, wparam, lparam) }
}

fn own_window() -> Option<HWND> {
    let raw = MAIN_WINDOW.load(Ordering::Relaxed);
    (raw != 0).then_some(HWND(raw as *mut std::ffi::c_void))
}

/// Switch off the open and close animation for our own window.
///
/// Without this, hiding the window before a capture leaves a half transparent
/// ghost of it in the shot, because the fade is still running when the screen
/// is read.
pub fn disable_window_animations() {
    let Some(hwnd) = own_window() else {
        return;
    };
    unsafe {
        let on: BOOL = true.into();
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_TRANSITIONS_FORCEDISABLED,
            (&raw const on).cast(),
            size_of::<BOOL>() as u32,
        );
    }
}

/// Everything needed to put the window back exactly as it was before a
/// capture: position and show state, where it sat in the stack of windows,
/// and which window was in front.
pub struct Placement {
    window: WINDOWPLACEMENT,
    /// The window directly above ours, so ours can go back underneath it.
    above: isize,
    /// Whoever had the keyboard, so they can have it back.
    foreground: isize,
}

// Plain data; only the handles it names are tied to a thread.
unsafe impl Send for Placement {}

/// Remember the window before a capture touches it.
///
/// Sending a size, a position and a minimise as three separate commands does
/// not survive the round trip: Windows only records a normal position for a
/// window that is not minimised at that moment, so the position is lost and
/// the window comes back as a stub in the corner. A placement carries both at
/// once. The stacking order and the foreground window are kept alongside, so a
/// window that was sitting in the background stays in the background.
pub fn save_window_placement() -> Option<Placement> {
    let hwnd = own_window()?;
    unsafe {
        let mut window = WINDOWPLACEMENT {
            length: size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        GetWindowPlacement(hwnd, &mut window).ok()?;
        let above = GetWindow(hwnd, GW_HWNDPREV)
            .map(|h| h.0 as isize)
            .unwrap_or(0);
        let foreground = GetForegroundWindow().0 as isize;
        Some(Placement {
            window,
            above,
            foreground,
        })
    }
}

/// Put the window back without activating it, back into its old slot in the
/// stack, and hand the keyboard back to whoever had it.
pub fn restore_window_placement(placement: &Placement, shown: bool, minimized: bool) -> bool {
    let Some(hwnd) = own_window() else {
        return false;
    };
    unsafe {
        let mut window = placement.window;
        // The plain "show" commands activate the window, which is exactly
        // what pulled a background window to the front after every shot.
        window.showCmd = if !shown {
            SW_HIDE.0 as u32
        } else if minimized {
            // Not SW_SHOWMINNOACTIVE: a window minimised that way comes back
            // from the task bar as a 6x6 stub in the corner. A minimised window
            // is not in anyone's way, and the keyboard is handed back below.
            SW_SHOWMINIMIZED.0 as u32
        } else if window.showCmd == SW_SHOWMAXIMIZED.0 as u32 {
            SW_SHOWMAXIMIZED.0 as u32
        } else {
            SW_SHOWNOACTIVATE.0 as u32
        };
        if SetWindowPlacement(hwnd, &window).is_err() {
            return false;
        }

        if shown && !minimized {
            let above = HWND(placement.above as *mut std::ffi::c_void);
            // Slotting in under a topmost window would make ours topmost as
            // well; in that case it was at the top of the ordinary windows
            // anyway, which is where the overlay left it.
            let usable = placement.above != 0
                && IsWindow(Some(above)).as_bool()
                && (GetWindowLongW(above, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0) == 0;
            if usable {
                let _ = SetWindowPos(
                    hwnd,
                    Some(above),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
        }

        let foreground = HWND(placement.foreground as *mut std::ffi::c_void);
        if placement.foreground != 0 && IsWindow(Some(foreground)).as_bool() {
            let _ = SetForegroundWindow(foreground);
        }
        true
    }
}

/// Whether our own window is still on screen. The capture waits for this to
/// turn false rather than guessing how long hiding takes.
pub fn own_window_is_visible() -> bool {
    match own_window() {
        Some(hwnd) => unsafe { IsWindowVisible(hwnd).as_bool() },
        None => false,
    }
}

pub fn foreground_window() -> Option<WindowInfo> {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_invalid() {
        return None;
    }
    let collector = Collector {
        windows: Vec::new(),
        screen: virtual_screen(),
        own_pid: std::process::id(),
    };
    unsafe { describe_window(hwnd, &collector) }
}

// ------------------------------------------------------------------ time ----

pub fn local_now() -> LocalTime {
    let st = unsafe { GetLocalTime() };
    LocalTime {
        year: st.wYear as i32,
        month: st.wMonth as u32,
        day: st.wDay as u32,
        hour: st.wHour as u32,
        minute: st.wMinute as u32,
        second: st.wSecond as u32,
        millis: st.wMilliseconds as u32,
    }
}

pub fn local_from_system_time(t: SystemTime) -> LocalTime {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    // Windows file times count 100 ns ticks from 1601.
    let ticks = (secs + 11_644_473_600) as u64 * 10_000_000;
    let ft = FILETIME {
        dwLowDateTime: (ticks & 0xFFFF_FFFF) as u32,
        dwHighDateTime: (ticks >> 32) as u32,
    };
    unsafe {
        let mut utc = Default::default();
        if FileTimeToSystemTime(&ft, &mut utc).is_err() {
            return local_now();
        }
        let mut local = Default::default();
        if SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_err() {
            return local_now();
        }
        LocalTime {
            year: local.wYear as i32,
            month: local.wMonth as u32,
            day: local.wDay as u32,
            hour: local.wHour as u32,
            minute: local.wMinute as u32,
            second: local.wSecond as u32,
            millis: local.wMilliseconds as u32,
        }
    }
}

// ------------------------------------------------------------ drag source ----

#[implement(IDropSource)]
struct DropSource;

#[allow(non_snake_case)]
impl IDropSource_Impl for DropSource_Impl {
    fn QueryContinueDrag(&self, escape_pressed: BOOL, key_state: MODIFIERKEYS_FLAGS) -> HRESULT {
        const DRAGDROP_S_DROP: HRESULT = HRESULT(0x0004_0100u32 as i32);
        const DRAGDROP_S_CANCEL: HRESULT = HRESULT(0x0004_0101u32 as i32);
        if escape_pressed.as_bool() {
            DRAGDROP_S_CANCEL
        } else if key_state & MK_LBUTTON == MODIFIERKEYS_FLAGS(0) {
            DRAGDROP_S_DROP
        } else {
            windows::Win32::Foundation::S_OK
        }
    }

    fn GiveFeedback(&self, _effect: DROPEFFECT) -> HRESULT {
        const DRAGDROP_S_USEDEFAULTCURSORS: HRESULT = HRESULT(0x0004_0102u32 as i32);
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

/// Start a real file drag, so dropping on Explorer, a chat window or an upload
/// field behaves exactly like dragging the file out of a folder.
///
/// The data object comes from the shell itself. Building one by hand means
/// implementing `IDataObject` plus a format enumerator; asking the shell for
/// the same object the file manager uses is shorter and more compatible.
pub fn start_file_drag(paths: &[PathBuf]) -> Result<(), String> {
    if paths.is_empty() {
        return Err("nothing to drag".into());
    }
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        // DoDragDrop needs full OLE on this thread, not just COM.
        let _ = OleInitialize(None);

        let mut pidls: Vec<*const ITEMIDLIST> = Vec::with_capacity(paths.len());
        for path in paths {
            let w = wide(path);
            let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
            if SHParseDisplayName(PCWSTR(w.as_ptr()), None, &mut pidl, 0, None).is_ok() {
                pidls.push(pidl);
            }
        }
        if pidls.is_empty() {
            return Err("file not found".into());
        }

        let result = (|| -> Result<(), String> {
            let items: IShellItemArray = SHCreateShellItemArrayFromIDLists(&pidls)
                .map_err(|e| format!("shell item array: {e}"))?;
            let data: IDataObject = items
                .BindToHandler(None, &BHID_DataObject)
                .map_err(|e| format!("data object: {e}"))?;
            let source: IDropSource = DropSource.into();
            let mut effect = DROPEFFECT::default();
            // Blocks until the mouse is released, like every other drag source.
            let _ = DoDragDrop(&data, &source, DROPEFFECT_COPY, &mut effect);
            Ok(())
        })();

        for pidl in pidls {
            CoTaskMemFree(Some(pidl.cast()));
        }
        result
    }
}

// ----------------------------------------------------------------- shell ----

pub fn reveal_in_file_manager(path: &Path) {
    let arg = wide(format!("/select,\"{}\"", path.display()));
    let verb = wide("open");
    let file = wide("explorer.exe");
    unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR(arg.as_ptr()),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

pub fn open_path(path: &Path) {
    let verb = wide("open");
    let file = wide(path);
    unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(file.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

/// Move files to the recycle bin instead of unlinking them. Deleting from the
/// history should never be the last word.
pub fn move_to_trash(paths: &[PathBuf]) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut buffer: Vec<u16> = Vec::new();
    for path in paths {
        buffer.extend(path.as_os_str().encode_wide());
        buffer.push(0);
    }
    buffer.push(0);

    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: PCWSTR(buffer.as_ptr()),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT).0 as u16,
        ..Default::default()
    };
    let code = unsafe { SHFileOperationW(&mut op) };
    if code == 0 {
        Ok(())
    } else {
        Err(format!("recycle bin refused the file (code {code})"))
    }
}

pub fn pick_folder(start: &Path) -> Option<PathBuf> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let dialog: IFileOpenDialog =
            CoCreateInstance(&FileOpenDialog, None, Default::default()).ok()?;
        dialog
            .SetOptions(dialog.GetOptions().ok()? | FOS_PICKFOLDERS)
            .ok()?;
        if start.exists() {
            let w = wide(start);
            if let Ok(item) =
                SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(w.as_ptr()), None)
            {
                let _ = dialog.SetFolder(&item);
            }
        }
        dialog.Show(None).ok()?;
        let item = dialog.GetResult().ok()?;
        let raw = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = PathBuf::from(OsString::from_wide(raw.as_wide()));
        CoTaskMemFree(Some(raw.0.cast()));
        Some(path)
    }
}

pub fn pictures_dir() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .map(|p| p.join("Pictures"))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn config_dir() -> PathBuf {
    std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Visura")
}

pub fn cache_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(config_dir)
        .join("Visura")
        .join("cache")
}

// ------------------------------------------------------------- autostart ----

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    unsafe {
        let subkey = wide(RUN_KEY);
        let mut key = HKEY::default();
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(subkey.as_ptr()),
            None,
            KEY_SET_VALUE,
            &mut key,
        )
        .ok()
        .map_err(|e| format!("registry: {e}"))?;

        let name = wide("Visura");
        let result = if enabled {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let value = wide(format!("\"{}\" --background", exe.display()));
            let bytes = std::slice::from_raw_parts(value.as_ptr().cast::<u8>(), value.len() * 2);
            RegSetValueExW(key, PCWSTR(name.as_ptr()), None, REG_SZ, Some(bytes))
        } else {
            RegDeleteValueW(key, PCWSTR(name.as_ptr()))
        };
        let _ = RegCloseKey(key);
        result.ok().map_err(|e| format!("registry: {e}"))
    }
}

// -------------------------------------------------------- single instance ----

const MUTEX_NAME: &str = "Local\\VisuraSingleInstance";

/// One named event per request; an event carries no data, only that it fired.
/// The name for showing the window predates the others and is kept as it was.
fn event_name(request: Request) -> String {
    match request {
        Request::Show => "Local\\VisuraShowWindow".to_string(),
        other => format!("Local\\VisuraRequest-{}", other.name()),
    }
}

/// A handle that keeps the single instance claim alive for the whole run.
pub struct InstanceGuard(HANDLE);

// The handle is only closed when the process ends; moving it between threads
// is safe.
unsafe impl Send for InstanceGuard {}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Returns `None` when another copy already runs; that copy is handed the
/// request instead.
pub fn acquire_single_instance(request: Request) -> Option<InstanceGuard> {
    unsafe {
        let name = wide(MUTEX_NAME);
        let mutex = CreateMutexW(None, true, PCWSTR(name.as_ptr())).ok()?;
        if windows::Win32::Foundation::GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(mutex);
            let event_name = wide(event_name(request));
            if let Ok(event) = CreateEventW(None, false, false, PCWSTR(event_name.as_ptr())) {
                let _ = SetEvent(event);
                let _ = CloseHandle(event);
            }
            return None;
        }
        Some(InstanceGuard(mutex))
    }
}

/// Call `on_request` whenever another copy of the program is started and
/// hands this one a request. The guard is kept alive by the thread.
pub fn spawn_request_listener(guard: InstanceGuard, on_request: impl Fn(Request) + Send + 'static) {
    std::thread::Builder::new()
        .name("visura-instance".into())
        .spawn(move || {
            let _guard = guard;
            let mut events = Vec::new();
            for request in Request::ALL {
                let name = wide(event_name(request));
                match unsafe { CreateEventW(None, false, false, PCWSTR(name.as_ptr())) } {
                    Ok(event) => events.push(event),
                    Err(_) => return,
                }
            }
            loop {
                let fired = unsafe { WaitForMultipleObjects(&events, false, u32::MAX) };
                let index = fired.0.wrapping_sub(WAIT_OBJECT_0.0) as usize;
                match Request::ALL.get(index) {
                    Some(&request) => on_request(request),
                    // Failed or abandoned: nothing sensible left to wait for.
                    None => return,
                }
            }
        })
        .ok();
}
