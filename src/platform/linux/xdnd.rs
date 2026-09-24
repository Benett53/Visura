//! Dragging files out of Visura on X11, as the source side of the XDND
//! protocol (freedesktop.org, version 5).
//!
//! The drag runs on its own thread with its own X connection. It does not grab
//! the pointer: the button that started the drag is still held inside the
//! Visura window, so winit has an implicit grab that no other connection can
//! take over. None is needed either. The pointer is polled for position and
//! button state, and the drop target only ever talks to the source through
//! client messages and the `XdndSelection`, both of which work without a grab.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    AtomEnum, ClientMessageEvent, ConnectionExt, CreateWindowAux, EventMask, KeyButMask, PropMode,
    SELECTION_NOTIFY_EVENT, SelectionNotifyEvent, SelectionRequestEvent, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{CURRENT_TIME, NONE};

/// Highest protocol version spoken here.
const VERSION: u32 = 5;
/// Targets below this version predate the parts of the protocol used here.
const MIN_VERSION: u32 = 3;
const POLL: Duration = Duration::from_millis(15);
/// A drag nobody finishes, for example because the button release was lost,
/// is given up after this.
const DRAG_LIMIT: Duration = Duration::from_secs(120);
/// How long a target may take to answer a position after the button is up.
const STATUS_WAIT: Duration = Duration::from_millis(500);
/// How long a target may take to fetch the files and say it is done.
const FINISH_WAIT: Duration = Duration::from_secs(10);

/// Visura's own window. A drop back onto it is not offered.
static MAIN_WINDOW: AtomicU32 = AtomicU32::new(0);
/// Only one drag at a time.
static DRAGGING: AtomicBool = AtomicBool::new(false);

pub fn set_main_window(window: u32) {
    MAIN_WINDOW.store(window, Ordering::Relaxed);
}

struct Atoms {
    aware: u32,
    proxy: u32,
    enter: u32,
    position: u32,
    status: u32,
    leave: u32,
    drop: u32,
    finished: u32,
    selection: u32,
    type_list: u32,
    action_copy: u32,
    uri_list: u32,
    plain_text: u32,
    targets: u32,
}

impl Atoms {
    fn intern(conn: &RustConnection) -> Result<Self, String> {
        let get = |name: &str| -> Result<u32, String> {
            conn.intern_atom(false, name.as_bytes())
                .map_err(|e| e.to_string())?
                .reply()
                .map(|r| r.atom)
                .map_err(|e| e.to_string())
        };
        Ok(Self {
            aware: get("XdndAware")?,
            proxy: get("XdndProxy")?,
            enter: get("XdndEnter")?,
            position: get("XdndPosition")?,
            status: get("XdndStatus")?,
            leave: get("XdndLeave")?,
            drop: get("XdndDrop")?,
            finished: get("XdndFinished")?,
            selection: get("XdndSelection")?,
            type_list: get("XdndTypeList")?,
            action_copy: get("XdndActionCopy")?,
            uri_list: get("text/uri-list")?,
            plain_text: get("text/plain;charset=utf-8")?,
            targets: get("TARGETS")?,
        })
    }
}

/// The window a drop would go to.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Target {
    window: Window,
    /// Where messages are sent: the window itself or the proxy it names.
    inbox: Window,
    version: u32,
}

pub struct Drag {
    conn: RustConnection,
    root: Window,
    source: Window,
    atoms: Atoms,
    /// When the selection was taken; every message carries it.
    time: u32,
    uri_list: Vec<u8>,
    plain_text: Vec<u8>,
}

/// Set up everything that can fail before the drag is handed to its thread,
/// so a failure can still be reported to the user.
///
/// `uris` are complete `file://` URIs, `paths` the same files as plain text
/// for targets such as terminals that only take text.
pub fn prepare(uris: &[String], paths: &[String]) -> Result<Drag, String> {
    if DRAGGING.load(Ordering::SeqCst) {
        return Err("a drag is already running".into());
    }
    let (conn, screen_num) = x11rb::connect(None).map_err(|e| e.to_string())?;
    let root = conn
        .setup()
        .roots
        .get(screen_num)
        .ok_or("no X11 screen")?
        .root;
    let atoms = Atoms::intern(&conn)?;

    let source = conn.generate_id().map_err(|e| e.to_string())?;
    conn.create_window(
        0,
        source,
        root,
        -10,
        -10,
        1,
        1,
        0,
        WindowClass::INPUT_ONLY,
        0,
        &CreateWindowAux::new()
            .override_redirect(1)
            .event_mask(EventMask::PROPERTY_CHANGE),
    )
    .map_err(|e| e.to_string())?;
    let types = [atoms.uri_list, atoms.plain_text];
    conn.change_property32(
        PropMode::REPLACE,
        source,
        atoms.type_list,
        AtomEnum::ATOM,
        &types,
    )
    .map_err(|e| e.to_string())?;
    conn.flush().map_err(|e| e.to_string())?;

    // The property change just made comes back as an event carrying the
    // server time, which is the timestamp the selection rules ask for.
    let time = wait_for_time(&conn, source).unwrap_or(CURRENT_TIME);
    conn.set_selection_owner(source, atoms.selection, time)
        .map_err(|e| e.to_string())?;
    conn.flush().map_err(|e| e.to_string())?;

    Ok(Drag {
        conn,
        root,
        source,
        atoms,
        time,
        uri_list: uri_list(uris).into_bytes(),
        plain_text: paths.join("\n").into_bytes(),
    })
}

fn wait_for_time(conn: &RustConnection, source: Window) -> Option<u32> {
    let deadline = Instant::now() + Duration::from_millis(500);
    while Instant::now() < deadline {
        match conn.poll_for_event().ok()? {
            Some(Event::PropertyNotify(e)) if e.window == source => return Some(e.time),
            Some(_) => {}
            None => std::thread::sleep(Duration::from_millis(2)),
        }
    }
    None
}

/// The body of a `text/uri-list`: one URI per line, lines ending in CRLF.
fn uri_list(uris: &[String]) -> String {
    uris.iter().map(|uri| format!("{uri}\r\n")).collect()
}

/// Run the drag on a thread of its own.
pub fn start(drag: Drag) -> Result<(), String> {
    if DRAGGING.swap(true, Ordering::SeqCst) {
        return Err("a drag is already running".into());
    }
    std::thread::Builder::new()
        .name("visura-drag".into())
        .spawn(move || {
            drag.run();
            DRAGGING.store(false, Ordering::SeqCst);
        })
        .map(|_| ())
        .map_err(|e| {
            DRAGGING.store(false, Ordering::SeqCst);
            e.to_string()
        })
}

impl Drag {
    fn run(self) {
        let started = Instant::now();
        let mut target: Option<Target> = None;
        // A position is out and its status has not come back yet. The
        // protocol wants one answer per position before the next is sent.
        let mut waiting = false;
        let mut accepted = false;
        let mut sent_at: Option<(i16, i16)> = None;

        loop {
            self.handle_events(target, &mut waiting, &mut accepted, &mut None);

            let Some(pointer) = self
                .conn
                .query_pointer(self.root)
                .ok()
                .and_then(|c| c.reply().ok())
            else {
                break;
            };
            let position = (pointer.root_x, pointer.root_y);
            let released = !pointer.mask.contains(KeyButMask::BUTTON1);

            let under = self.target_under_pointer();
            if under != target {
                if let Some(old) = target {
                    self.send(old, self.atoms.leave, [0, 0, 0, 0]);
                }
                target = under;
                waiting = false;
                accepted = false;
                sent_at = None;
                if let Some(new) = target {
                    let flags = new.version << 24 | 1; // more types in XdndTypeList
                    self.send(
                        new,
                        self.atoms.enter,
                        [flags, self.atoms.uri_list, self.atoms.plain_text, 0],
                    );
                }
            }

            if let Some(t) = target
                && !waiting
                && sent_at != Some(position)
            {
                self.send_position(t, position);
                waiting = true;
                sent_at = Some(position);
            }

            if released || started.elapsed() > DRAG_LIMIT {
                break;
            }
            std::thread::sleep(POLL);
        }

        if let Some(t) = target {
            // The answer to the last position decides whether a drop is
            // wanted at all.
            let deadline = Instant::now() + STATUS_WAIT;
            while waiting && Instant::now() < deadline {
                self.handle_events(target, &mut waiting, &mut accepted, &mut None);
                std::thread::sleep(Duration::from_millis(5));
            }
            if accepted {
                self.send(t, self.atoms.drop, [0, self.time, 0, 0]);
                let deadline = Instant::now() + FINISH_WAIT;
                let mut finished = Some(false);
                while finished == Some(false) && Instant::now() < deadline {
                    self.handle_events(target, &mut waiting, &mut accepted, &mut finished);
                    std::thread::sleep(Duration::from_millis(5));
                }
            } else {
                self.send(t, self.atoms.leave, [0, 0, 0, 0]);
            }
        }

        let _ = self.conn.destroy_window(self.source);
        let _ = self.conn.flush();
    }

    /// Answer requests for the data and note status replies. `finished`
    /// turns `Some(true)` once the target says the drop is complete.
    fn handle_events(
        &self,
        target: Option<Target>,
        waiting: &mut bool,
        accepted: &mut bool,
        finished: &mut Option<bool>,
    ) {
        while let Ok(Some(event)) = self.conn.poll_for_event() {
            match event {
                Event::ClientMessage(e) => {
                    let data = e.data.as_data32();
                    let from_target = target.is_some_and(|t| t.window == data[0]);
                    if e.type_ == self.atoms.status && from_target {
                        *waiting = false;
                        *accepted = data[1] & 1 != 0;
                    } else if e.type_ == self.atoms.finished && from_target {
                        *finished = Some(true);
                    }
                }
                Event::SelectionRequest(e) => self.answer(e),
                _ => {}
            }
        }
    }

    /// Hand the files over to whoever asks for the selection.
    fn answer(&self, e: SelectionRequestEvent) {
        // Clients older than ICCCM 2 leave the property empty.
        let property = if e.property == NONE {
            e.target
        } else {
            e.property
        };
        let stored = if e.selection != self.atoms.selection {
            false
        } else if e.target == self.atoms.uri_list || e.target == self.atoms.plain_text {
            let data = if e.target == self.atoms.uri_list {
                &self.uri_list
            } else {
                &self.plain_text
            };
            self.conn
                .change_property8(PropMode::REPLACE, e.requestor, property, e.target, data)
                .is_ok()
        } else if e.target == self.atoms.targets {
            let offered = [
                self.atoms.targets,
                self.atoms.uri_list,
                self.atoms.plain_text,
            ];
            self.conn
                .change_property32(
                    PropMode::REPLACE,
                    e.requestor,
                    property,
                    AtomEnum::ATOM,
                    &offered,
                )
                .is_ok()
        } else {
            false
        };
        let notify = SelectionNotifyEvent {
            response_type: SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: e.time,
            requestor: e.requestor,
            selection: e.selection,
            target: e.target,
            property: if stored { property } else { NONE },
        };
        let _ = self
            .conn
            .send_event(false, e.requestor, EventMask::NO_EVENT, notify);
        let _ = self.conn.flush();
    }

    /// Walk down from the root to the window under the pointer and stop at
    /// the first one that takes drops. With a reparenting window manager that
    /// is the program's window inside the frame.
    fn target_under_pointer(&self) -> Option<Target> {
        let mut window = self.root;
        for _ in 0..16 {
            let child = self.conn.query_pointer(window).ok()?.reply().ok()?.child;
            if child == NONE {
                return None;
            }
            if let Some(version) = self.aware_version(child) {
                if version < MIN_VERSION || child == MAIN_WINDOW.load(Ordering::Relaxed) {
                    return None;
                }
                let inbox = self.proxy_of(child).unwrap_or(child);
                return Some(Target {
                    window: child,
                    inbox,
                    version: version.min(VERSION),
                });
            }
            window = child;
        }
        None
    }

    fn aware_version(&self, window: Window) -> Option<u32> {
        let reply = self
            .conn
            .get_property(false, window, self.atoms.aware, AtomEnum::ATOM, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        reply.value32()?.next()
    }

    fn proxy_of(&self, window: Window) -> Option<Window> {
        let reply = self
            .conn
            .get_property(false, window, self.atoms.proxy, AtomEnum::WINDOW, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        reply.value32()?.next().filter(|&w| w != NONE)
    }

    fn send_position(&self, target: Target, (x, y): (i16, i16)) {
        let packed = ((x as u16 as u32) << 16) | (y as u16 as u32);
        self.send(
            target,
            self.atoms.position,
            [0, packed, self.time, self.atoms.action_copy],
        );
    }

    /// Send one XDND message. The first data word is always the source.
    fn send(&self, target: Target, kind: u32, rest: [u32; 4]) {
        let data = [self.source, rest[0], rest[1], rest[2], rest[3]];
        let message = ClientMessageEvent::new(32, target.window, kind, data);
        let _ = self
            .conn
            .send_event(false, target.inbox, EventMask::NO_EVENT, message);
        let _ = self.conn.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_lists_end_every_line_with_crlf() {
        let uris = ["file:///a.png".to_string(), "file:///b%20c.png".to_string()];
        assert_eq!(uri_list(&uris), "file:///a.png\r\nfile:///b%20c.png\r\n");
    }
}
