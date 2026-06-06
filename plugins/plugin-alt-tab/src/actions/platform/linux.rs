use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;

mod trace;

pub fn activate_window(window_id: u32) {
    let Some((conn, root)) = connect() else {
        return;
    };
    trace::activating(&conn, window_id);
    let Some(atom) = intern(&conn, b"_NET_ACTIVE_WINDOW") else {
        return;
    };
    send_to_root(&conn, root, window_id, atom, [2, 0, 0, 0, 0]);
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
