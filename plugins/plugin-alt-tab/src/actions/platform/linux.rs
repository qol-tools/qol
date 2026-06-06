use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

mod trace;

const REASSERT_STEPS_MS: [u64; 5] = [16, 24, 40, 60, 100];

static ACTIVATOR: OnceLock<Sender<u32>> = OnceLock::new();

pub fn activate_window(window_id: u32) {
    if window_id == 0 {
        return;
    }
    let _ = ACTIVATOR.get_or_init(start_activator).send(window_id);
}

fn start_activator() -> Sender<u32> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || run_activator(rx));
    tx
}

fn run_activator(rx: Receiver<u32>) {
    let Some(activator) = Activator::new() else {
        return;
    };
    let Ok(mut target) = rx.recv() else {
        return;
    };
    loop {
        while let Ok(newer) = rx.try_recv() {
            target = newer;
        }
        match activator.drive(target, &rx) {
            Some(newer) => target = newer,
            None => {
                let Ok(next) = rx.recv() else {
                    return;
                };
                target = next;
            }
        }
    }
}

struct Activator {
    conn: RustConnection,
    root: u32,
    active_atom: u32,
    baseline_ms: u32,
    baseline_at: Instant,
}

impl Activator {
    fn new() -> Option<Self> {
        let (conn, screen_num) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots[screen_num].root;
        let active_atom = intern(&conn, b"_NET_ACTIVE_WINDOW")?;
        let (baseline_ms, baseline_at) = probe_server_time(&conn, root)?;
        Some(Self {
            conn,
            root,
            active_atom,
            baseline_ms,
            baseline_at,
        })
    }

    fn drive(&self, target: u32, rx: &Receiver<u32>) -> Option<u32> {
        let time = self.send_activate(target);
        trace::activating(&self.conn, target, time);
        for step_ms in REASSERT_STEPS_MS {
            match rx.recv_timeout(Duration::from_millis(step_ms)) {
                Ok(newer) => return Some(newer),
                Err(RecvTimeoutError::Disconnected) => return None,
                Err(RecvTimeoutError::Timeout) => {}
            }
            if self.active_window() == Some(target) {
                return None;
            }
            self.send_activate(target);
        }
        None
    }

    fn send_activate(&self, window_id: u32) -> u32 {
        let time = self.server_now();
        let event = ClientMessageEvent::new(32, window_id, self.active_atom, [2, time, 0, 0, 0]);
        let mask = EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT;
        let _ = self.conn.send_event(false, self.root, mask, event);
        let _ = self.conn.flush();
        time
    }

    fn active_window(&self) -> Option<u32> {
        let reply = self
            .conn
            .get_property(false, self.root, self.active_atom, AtomEnum::WINDOW, 0, 1)
            .ok()?
            .reply()
            .ok()?;
        let mut values = reply.value32()?;
        values.next()
    }

    fn server_now(&self) -> u32 {
        let elapsed = self.baseline_at.elapsed().as_millis() as u32;
        self.baseline_ms.wrapping_add(elapsed)
    }
}

fn probe_server_time(conn: &impl Connection, root: u32) -> Option<(u32, Instant)> {
    let window = conn.generate_id().ok()?;
    conn.create_window(
        0,
        window,
        root,
        -100,
        -100,
        1,
        1,
        0,
        WindowClass::INPUT_ONLY,
        x11rb::COPY_FROM_PARENT,
        &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
    )
    .ok()?;
    let probe = intern(conn, b"_QOL_ALT_TAB_TIME")?;
    let empty: &[u8] = &[];
    conn.change_property8(PropMode::APPEND, window, probe, AtomEnum::STRING, empty)
        .ok()?;
    conn.flush().ok()?;

    let time = loop {
        match conn.wait_for_event().ok()? {
            Event::PropertyNotify(notify) if notify.window == window => break notify.time,
            _ => continue,
        }
    };
    let _ = conn.destroy_window(window);
    let _ = conn.flush();
    Some((time, Instant::now()))
}

pub fn close_window(window_id: u32) {
    let Some((conn, root)) = connect() else {
        return;
    };
    let Some(atom) = intern(&conn, b"_NET_CLOSE_WINDOW") else {
        return;
    };
    send_to_root(&conn, root, window_id, atom, [0, 2, 0, 0, 0]);
}

pub fn quit_app(window_id: u32) {
    let Some((conn, _)) = connect() else { return };
    let Some(pid_atom) = intern(&conn, b"_NET_WM_PID") else {
        return;
    };
    let Ok(reply) = conn.get_property(false, window_id, pid_atom, AtomEnum::CARDINAL, 0, 1) else {
        return;
    };
    let Ok(prop) = reply.reply() else { return };
    let Some(pid) = prop.value32().and_then(|mut v| v.next()) else {
        return;
    };
    unsafe {
        libc::kill(pid as i32, libc::SIGTERM);
    }
}

pub fn minimize_window_by_id(window_id: u32) {
    let Some((conn, root)) = connect() else {
        return;
    };
    let Some(atom) = intern(&conn, b"WM_CHANGE_STATE") else {
        return;
    };
    send_to_root(&conn, root, window_id, atom, [3, 0, 0, 0, 0]);
}

fn connect() -> Option<(RustConnection, u32)> {
    let (conn, screen_num) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots[screen_num].root;
    Some((conn, root))
}

fn intern(conn: &impl Connection, name: &[u8]) -> Option<u32> {
    conn.intern_atom(false, name)
        .ok()?
        .reply()
        .ok()
        .map(|r| r.atom)
}

fn send_to_root(conn: &impl Connection, root: u32, window: u32, message_type: u32, data: [u32; 5]) {
    let event = ClientMessageEvent::new(32, window, message_type, data);
    let mask = EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT;
    let _ = conn.send_event(false, root, mask, event);
    let _ = conn.flush();
}
