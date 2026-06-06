use std::sync::atomic::{AtomicU64, Ordering};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::protocol::Event;
use x11rb::wrapper::ConnectionExt as _;

mod trace;

static ACTIVATE_GEN: AtomicU64 = AtomicU64::new(0);

pub fn activate_window(window_id: u32) {
    if window_id == 0 {
        return;
    }
    let generation = ACTIVATE_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let Some((conn, root)) = connect() else {
        return;
    };
    let time = send_activate(&conn, root, window_id).unwrap_or(0);
    trace::activating(&conn, window_id, time);
    spawn_reassert(window_id, generation);
}

fn send_activate(conn: &impl Connection, root: u32, window_id: u32) -> Option<u32> {
    let atom = intern(conn, b"_NET_ACTIVE_WINDOW")?;
    let time = server_time(conn, root).unwrap_or(0);
    send_to_root(conn, root, window_id, atom, [2, time, 0, 0, 0]);
    Some(time)
}

fn spawn_reassert(window_id: u32, generation: u64) {
    qol_gpui::platform::spawn_reassert_driver(
        &ACTIVATE_GEN,
        generation,
        &[16u64, 24, 40, 60, 100],
        move || {
            let Some((conn, root)) = connect() else {
                return true;
            };
            active_window(&conn, root) == Some(window_id)
        },
        move || {
            let Some((conn, root)) = connect() else {
                return;
            };
            let _ = send_activate(&conn, root, window_id);
        },
    );
}

fn active_window(conn: &impl Connection, root: u32) -> Option<u32> {
    let atom = intern(conn, b"_NET_ACTIVE_WINDOW")?;
    let reply = conn
        .get_property(false, root, atom, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    let mut values = reply.value32()?;
    values.next()
}

fn server_time(conn: &impl Connection, root: u32) -> Option<u32> {
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
    Some(time)
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

fn connect() -> Option<(x11rb::rust_connection::RustConnection, u32)> {
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
