use std::sync::{Mutex, OnceLock};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

pub fn should_poll_process_focus() -> bool {
    matches!(
        qol_platform::linux_display_backend(),
        qol_platform::LinuxDisplayBackend::X11
    )
}

pub fn has_process_focus() -> bool {
    if !should_poll_process_focus() {
        return true;
    }

    static X11_CONN: OnceLock<Mutex<Option<RustConnection>>> = OnceLock::new();

    let conn_mutex = X11_CONN.get_or_init(|| Mutex::new(x11rb::connect(None).map(|(c, _)| c).ok()));

    let mut guard = match conn_mutex.lock() {
        Ok(g) => g,
        Err(_) => return true,
    };

    let focus_opt = {
        let conn = match &*guard {
            Some(c) => c,
            None => {
                *guard = x11rb::connect(None).map(|(c, _)| c).ok();
                if guard.is_none() {
                    return true;
                }
                guard.as_ref().unwrap()
            }
        };
        conn.get_input_focus()
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|reply| reply.focus)
    };

    let focus = match focus_opt {
        Some(f) => f,
        None => {
            *guard = None;
            return true;
        }
    };

    if focus == 0 {
        return false;
    }

    owns_window(guard.as_ref().unwrap(), focus, std::process::id())
}

fn owns_window(conn: &impl Connection, mut window: u32, target_pid: u32) -> bool {
    loop {
        if window_pid(conn, window) == Some(target_pid) {
            return true;
        }

        let Ok(reply) = conn.query_tree(window) else {
            return false;
        };
        let Ok(tree) = reply.reply() else {
            return false;
        };
        if tree.parent == 0 || tree.parent == tree.root || tree.parent == window {
            return false;
        }

        window = tree.parent;
    }
}

fn window_pid(conn: &impl Connection, window: u32) -> Option<u32> {
    let atom = conn
        .intern_atom(false, b"_NET_WM_PID")
        .ok()?
        .reply()
        .ok()?
        .atom;
    let prop = conn
        .get_property(false, window, atom, AtomEnum::CARDINAL, 0, 1)
        .ok()?
        .reply()
        .ok()?;

    prop.value32().and_then(|mut value| value.next())
}
