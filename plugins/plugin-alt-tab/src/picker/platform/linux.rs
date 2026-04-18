use std::sync::{Mutex, OnceLock};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

fn keymap_conn() -> &'static Mutex<Option<RustConnection>> {
    static CONN: OnceLock<Mutex<Option<RustConnection>>> = OnceLock::new();
    CONN.get_or_init(|| Mutex::new(x11rb::connect(None).map(|(c, _)| c).ok()))
}

fn query_keymap_keys() -> Option<[u8; 32]> {
    let mut guard = keymap_conn().lock().ok()?;
    let keys = {
        let conn = guard.as_ref()?;
        conn.query_keymap().ok()?.reply().ok().map(|r| r.keys)
    };
    if keys.is_none() {
        *guard = x11rb::connect(None).map(|(c, _)| c).ok();
    }
    keys
}

pub fn picker_window_kind() -> gpui::WindowKind {
    gpui::WindowKind::PopUp
}

pub fn dismiss_picker(window: &mut gpui::Window) {
    window.remove_window();
}

pub fn is_modifier_held() -> bool {
    let Some(keys) = query_keymap_keys() else {
        return false;
    };
    let alt_l_held = keys[64 / 8] & (1 << (64 % 8)) != 0;
    let alt_r_held = keys[108 / 8] & (1 << (108 % 8)) != 0;
    alt_l_held || alt_r_held
}

#[allow(dead_code)]
pub fn is_shift_held() -> bool {
    let Some(keys) = query_keymap_keys() else {
        return false;
    };
    let shift_l = keys[50 / 8] & (1 << (50 % 8)) != 0;
    let shift_r = keys[62 / 8] & (1 << (62 % 8)) != 0;
    shift_l || shift_r
}

pub fn set_accessory_policy() {}

pub fn reposition_picker_window(x: f64, y: f64) -> bool {
    move_window_by_title("qol-alt-tab-picker", x as i32, y as i32)
}

fn move_window_by_title(title: &str, x: i32, y: i32) -> bool {
    let Ok((conn, screen_num)) = x11rb::connect(None) else {
        return false;
    };
    let root = conn.setup().roots[screen_num].root;
    let list_atom = intern(&conn, b"_NET_CLIENT_LIST");
    let name_atom = intern(&conn, b"_NET_WM_NAME");
    let utf8_atom = intern(&conn, b"UTF8_STRING");
    let (Some(list_atom), Some(name_atom), Some(utf8_atom)) = (list_atom, name_atom, utf8_atom)
    else {
        return false;
    };

    let Ok(reply) = conn.get_property(false, root, list_atom, AtomEnum::WINDOW, 0, 1024) else {
        return false;
    };
    let Ok(prop) = reply.reply() else {
        return false;
    };
    let Some(ids) = prop.value32() else {
        return false;
    };

    for wid in ids {
        let Ok(name_reply) = conn.get_property(false, wid, name_atom, utf8_atom, 0, 256) else {
            continue;
        };
        let Ok(name_prop) = name_reply.reply() else {
            continue;
        };
        if String::from_utf8_lossy(&name_prop.value).contains(title) {
            let aux = ConfigureWindowAux::new().x(x).y(y);
            let _ = conn.configure_window(wid, &aux);
            let _ = conn.flush();
            return true;
        }
    }
    false
}

fn intern(conn: &impl Connection, name: &[u8]) -> Option<u32> {
    conn.intern_atom(false, name)
        .ok()?
        .reply()
        .ok()
        .map(|r| r.atom)
}

pub fn disable_window_shadow() {}
