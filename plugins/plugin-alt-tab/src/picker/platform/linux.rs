use std::sync::{Mutex, OnceLock};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

const BOOTSTRAP_X: f64 = 0.0;
const BOOTSTRAP_Y: f64 = 0.0;

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
    window.minimize_window();
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

pub fn picker_backing_scale() -> Option<f32> {
    None
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

    let Some(wid) = find_window_by_title(&conn, root, list_atom, name_atom, utf8_atom, title)
    else {
        return false;
    };
    move_window(&conn, root, wid, x, y)
}

fn find_window_by_title(
    conn: &impl Connection,
    root: u32,
    list_atom: u32,
    name_atom: u32,
    utf8_atom: u32,
    title: &str,
) -> Option<u32> {
    let mut ids = root_window_ids(conn, root, list_atom);
    append_tree_window_ids(conn, root, &mut ids);

    ids.into_iter().find(|&wid| {
        window_title_matches(conn, wid, name_atom, utf8_atom, title)
            || window_title_matches(
                conn,
                wid,
                AtomEnum::WM_NAME.into(),
                AtomEnum::ANY.into(),
                title,
            )
    })
}

fn window_title_matches(
    conn: &impl Connection,
    wid: u32,
    name_atom: u32,
    ty: u32,
    title: &str,
) -> bool {
    let Ok(name_reply) = conn.get_property(false, wid, name_atom, ty, 0, 256) else {
        return false;
    };
    let Ok(name_prop) = name_reply.reply() else {
        return false;
    };
    String::from_utf8_lossy(&name_prop.value).contains(title)
}

fn move_window(conn: &impl Connection, root: u32, wid: u32, x: i32, y: i32) -> bool {
    let aux = ConfigureWindowAux::new().x(x).y(y);
    let _ = conn.configure_window(wid, &aux);

    if let Some(atom) = intern(conn, b"_NET_MOVERESIZE_WINDOW") {
        const MOVERESIZE_X: u32 = 1 << 8;
        const MOVERESIZE_Y: u32 = 1 << 9;
        let event = ClientMessageEvent::new(
            32,
            wid,
            atom,
            [MOVERESIZE_X | MOVERESIZE_Y, x as u32, y as u32, 0, 0],
        );
        let mask = EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT;
        let _ = conn.send_event(false, root, mask, event);
    }
    let _ = conn.flush();
    true
}

fn root_window_ids(conn: &impl Connection, root: u32, list_atom: u32) -> Vec<u32> {
    let Ok(reply) = conn.get_property(false, root, list_atom, AtomEnum::WINDOW, 0, 1024) else {
        return Vec::new();
    };
    let Ok(prop) = reply.reply() else {
        return Vec::new();
    };
    prop.value32().map(|ids| ids.collect()).unwrap_or_default()
}

fn append_tree_window_ids(conn: &impl Connection, root: u32, ids: &mut Vec<u32>) {
    let Ok(reply) = conn.query_tree(root) else {
        return;
    };
    let Ok(tree) = reply.reply() else {
        return;
    };
    for child in tree.children {
        if !ids.contains(&child) {
            ids.push(child);
        }
    }
}

fn intern(conn: &impl Connection, name: &[u8]) -> Option<u32> {
    conn.intern_atom(false, name)
        .ok()?
        .reply()
        .ok()
        .map(|r| r.atom)
}

pub fn disable_window_shadow() {}

pub fn set_ghost_opacity(_opacity: Option<f32>) {}

pub fn show_picker() {}

pub fn hide_picker() {
    let _ = minimize_window_by_title("qol-alt-tab-picker");
}

pub fn offscreen_origin() -> (f64, f64) {
    (BOOTSTRAP_X, BOOTSTRAP_Y)
}

pub fn pre_create(
    config: &crate::config::AltTabConfig,
    current: &crate::PickerWindowState,
    cx: &mut gpui::App,
) {
    crate::picker::create::pre_create_offscreen(config, current, cx)
}

fn minimize_window_by_title(title: &str) -> bool {
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
    let Some(wid) = find_window_by_title(&conn, root, list_atom, name_atom, utf8_atom, title)
    else {
        return false;
    };
    let Some(atom) = intern(&conn, b"WM_CHANGE_STATE") else {
        return false;
    };
    const WINDOW_ICONIC_STATE: u32 = 3;
    let event = ClientMessageEvent::new(32, wid, atom, [WINDOW_ICONIC_STATE, 0, 0, 0, 0]);
    let mask = EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT;
    let _ = conn.send_event(false, root, mask, event);
    let _ = conn.flush();
    true
}

pub fn destroy_non_target_windows(
    current: &crate::PickerWindowState,
    target: qol_plugin_api::window::MonitorKey,
    cx: &mut gpui::App,
) {
    current.borrow_mut().destroy_non_target(target, cx);
}

pub fn discard_old_window(
    current: &crate::PickerWindowState,
    target: qol_plugin_api::window::MonitorKey,
    handle: gpui::WindowHandle<crate::app::AltTabApp>,
    cx: &mut gpui::App,
) {
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/open] closing old window — will recreate on correct monitor");
    let _ = handle.update(cx, |_, window, _| window.remove_window());
    current.borrow_mut().remove(target);
}
