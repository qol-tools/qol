use x11rb::connection::Connection;
use x11rb::protocol::xproto::*;
use x11rb::wrapper::ConnectionExt as _;

pub fn reposition_window_by_title(title: &str, gpui_x: f64, gpui_y: f64) -> bool {
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
    move_window(&conn, root, wid, gpui_x as i32, gpui_y as i32)
}

pub fn hide_window_by_title(title: &str) -> bool {
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
    let Some(state_atom) = intern(&conn, b"WM_CHANGE_STATE") else {
        return false;
    };
    const WINDOW_ICONIC_STATE: u32 = 3;
    let event = ClientMessageEvent::new(32, wid, state_atom, [WINDOW_ICONIC_STATE, 0, 0, 0, 0]);
    let mask = EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT;
    let _ = conn.send_event(false, root, mask, event);
    let _ = conn.flush();
    true
}

/// Reverse of [`hide_window_by_title`]. Sends `_NET_ACTIVE_WINDOW` so the
/// WM deiconifies and raises the window in one round trip.
pub fn show_window_by_title(title: &str) -> bool {
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
    let Some(active_atom) = intern(&conn, b"_NET_ACTIVE_WINDOW") else {
        return false;
    };
    const SOURCE_APPLICATION: u32 = 1;
    let event = ClientMessageEvent::new(32, wid, active_atom, [SOURCE_APPLICATION, 0, 0, 0, 0]);
    let mask = EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT;
    let _ = conn.send_event(false, root, mask, event);
    let _ = conn.flush();
    true
}

/// X11 shadows are window-manager driven; no per-window API. Returns
/// `true` so callers can treat it uniformly with macOS.
pub fn disable_window_shadow(_title: &str) -> bool {
    true
}

pub fn configure_popup_window(title: &str) -> bool {
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

    // Mark the window as a utility so the WM does not treat a monitor-spanning
    // borderless window as fullscreen and auto-hide panels/docks.
    let type_atom = intern(&conn, b"_NET_WM_WINDOW_TYPE");
    let utility_atom = intern(&conn, b"_NET_WM_WINDOW_TYPE_UTILITY");
    if let (Some(type_atom), Some(utility_atom)) = (type_atom, utility_atom) {
        let _ = conn.change_property32(
            PropMode::REPLACE,
            wid,
            type_atom,
            AtomEnum::ATOM,
            &[utility_atom],
        );
    }
    let _ = conn.flush();
    true
}

/// Debug ghost visualisation is macOS-only (alpha compositing); X11 hide
/// iconifies the window, so there is nothing to tint. No-op.
pub fn set_ghost_debug(_opacity: Option<f32>, _color_hex: Option<&str>) {}

/// No per-window backing scale to query here; GPUI manages scaling itself, so
/// there is nothing to resync. Returns `None`.
pub fn window_backing_scale(_title: &str) -> Option<f32> {
    None
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
    String::from_utf8_lossy(&name_prop.value).trim_end_matches('\0') == title
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
