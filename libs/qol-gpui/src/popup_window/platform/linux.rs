use x11rb::connection::Connection;
#[cfg(debug_assertions)]
use x11rb::protocol::shape;
use x11rb::protocol::xproto::*;
use x11rb::wrapper::ConnectionExt as _;

#[cfg(debug_assertions)]
use std::sync::atomic::{AtomicU32, Ordering};

#[cfg(debug_assertions)]
static GHOST_DEBUG_ALPHA: AtomicU32 = AtomicU32::new(0);

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

pub fn set_window_bounds_by_title(
    title: &str,
    gpui_x: f64,
    gpui_y: f64,
    gpui_width: f64,
    gpui_height: f64,
) -> bool {
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
    set_window_bounds(
        &conn,
        root,
        wid,
        gpui_x.round() as i32,
        gpui_y.round() as i32,
        gpui_width.round().max(1.0) as u32,
        gpui_height.round().max(1.0) as u32,
    )
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
    #[cfg(debug_assertions)]
    {
        let opacity = ghost_debug_opacity();
        if compositor_running(&conn, screen_num) && set_input_passthrough(&conn, wid, true) {
            let _ = set_window_opacity(&conn, wid, opacity);
            let _ = conn.map_window(wid);
            let _ = conn.flush();
            return true;
        }
    }
    let Some(state_atom) = intern(&conn, b"WM_CHANGE_STATE") else {
        return false;
    };
    #[cfg(debug_assertions)]
    {
        let _ = clear_window_opacity(&conn, wid);
        let _ = set_input_passthrough(&conn, wid, false);
    }
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
    #[cfg(debug_assertions)]
    {
        let _ = clear_window_opacity(&conn, wid);
        let _ = set_input_passthrough(&conn, wid, false);
    }
    let _ = conn.map_window(wid);
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

    set_window_manager_decorations(&conn, wid, false);
    set_window_manager_state(&conn, wid);
    let _ = conn.flush();
    true
}

/// Debug-only: keep the hidden ghost mapped and faintly visible. X11 does not
/// have a native per-window alpha API, so this uses the EWMH compositor opacity
/// property and removes the input shape while hidden so the ghost cannot block
/// clicks. No-op in release builds.
pub fn set_ghost_debug(opacity: Option<f32>, _color_hex: Option<&str>) {
    #[cfg(debug_assertions)]
    GHOST_DEBUG_ALPHA.store(normalize_opacity(opacity).to_bits(), Ordering::Relaxed);
}

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

    let titled: Vec<u32> = ids
        .into_iter()
        .filter(|&wid| {
            window_title_matches(conn, wid, name_atom, utf8_atom, title)
                || window_title_matches(
                    conn,
                    wid,
                    AtomEnum::WM_NAME.into(),
                    AtomEnum::ANY.into(),
                    title,
                )
        })
        .collect();

    let pid_atom = intern(conn, b"_NET_WM_PID");
    let current_pid = std::process::id();
    let pid_match = pid_atom.and_then(|atom| {
        titled
            .iter()
            .copied()
            .find(|&wid| window_pid_matches(conn, wid, atom, current_pid))
    });
    #[cfg(debug_assertions)]
    if titled.len() > 1 {
        eprintln!(
            "[popup/x11] title={title:?} candidates={:?} pid_match={pid_match:?}",
            titled
        );
    }
    pid_match.or_else(|| titled.into_iter().next())
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

fn window_pid_matches(conn: &impl Connection, wid: u32, pid_atom: u32, pid: u32) -> bool {
    let Ok(reply) = conn.get_property(false, wid, pid_atom, AtomEnum::CARDINAL, 0, 1) else {
        return false;
    };
    let Ok(prop) = reply.reply() else {
        return false;
    };
    prop.value32()
        .and_then(|mut values| values.next())
        .map(|value| value == pid)
        .unwrap_or(false)
}

fn move_window(conn: &impl Connection, root: u32, wid: u32, x: i32, y: i32) -> bool {
    let target = top_level_frame(conn, root, wid).unwrap_or(wid);
    let aux = ConfigureWindowAux::new().x(x).y(y);
    let moved = conn.configure_window(target, &aux).is_ok();
    let _ = conn.flush();
    #[cfg(debug_assertions)]
    eprintln!("[popup/x11] move client={wid} target={target} root={root} to=({x},{y}) ok={moved}");
    moved
}

fn set_window_bounds(
    conn: &impl Connection,
    root: u32,
    wid: u32,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> bool {
    let target = top_level_frame(conn, root, wid).unwrap_or(wid);
    let aux = ConfigureWindowAux::new()
        .x(x)
        .y(y)
        .width(width)
        .height(height);
    let configured = conn.configure_window(target, &aux).is_ok();
    let _ = conn.flush();
    #[cfg(debug_assertions)]
    eprintln!(
        "[popup/x11] bounds client={wid} target={target} root={root} to=({x},{y}) size={}x{} ok={configured}",
        width, height
    );
    configured
}

fn top_level_frame(conn: &impl Connection, root: u32, wid: u32) -> Option<u32> {
    let mut child = wid;
    loop {
        let tree = conn.query_tree(child).ok()?.reply().ok()?;
        if tree.parent == root {
            return Some(child);
        }
        if tree.parent == 0 || tree.parent == child {
            return None;
        }
        child = tree.parent;
    }
}

fn set_window_manager_decorations(conn: &impl Connection, wid: u32, enabled: bool) {
    const MWM_HINTS_DECORATIONS: u32 = 1 << 1;
    let Some(atom) = intern(conn, b"_MOTIF_WM_HINTS") else {
        return;
    };
    let decorations = u32::from(enabled);
    let hints = [MWM_HINTS_DECORATIONS, 0, decorations, 0, 0];
    let _ = conn.change_property32(PropMode::REPLACE, wid, atom, atom, &hints);
}

fn set_window_manager_state(conn: &impl Connection, wid: u32) {
    let Some(state_atom) = intern(conn, b"_NET_WM_STATE") else {
        return;
    };
    let atoms = [
        intern(conn, b"_NET_WM_STATE_ABOVE"),
        intern(conn, b"_NET_WM_STATE_SKIP_TASKBAR"),
        intern(conn, b"_NET_WM_STATE_SKIP_PAGER"),
    ];
    let values: Vec<u32> = atoms.into_iter().flatten().collect();
    if values.is_empty() {
        return;
    }
    let _ = conn.change_property32(PropMode::REPLACE, wid, state_atom, AtomEnum::ATOM, &values);
}

#[cfg(debug_assertions)]
fn ghost_debug_opacity() -> f32 {
    f32::from_bits(GHOST_DEBUG_ALPHA.load(Ordering::Relaxed))
}

#[cfg(debug_assertions)]
fn normalize_opacity(opacity: Option<f32>) -> f32 {
    let value = opacity.unwrap_or(0.0);
    if !value.is_finite() {
        return 0.0;
    }
    value.clamp(0.0, 1.0)
}

#[cfg(debug_assertions)]
fn opacity_to_cardinal(opacity: f32) -> u32 {
    (normalize_opacity(Some(opacity)) * u32::MAX as f32).round() as u32
}

#[cfg(debug_assertions)]
fn set_window_opacity(conn: &impl Connection, wid: u32, opacity: f32) -> bool {
    let Some(atom) = intern(conn, b"_NET_WM_WINDOW_OPACITY") else {
        return false;
    };
    let value = opacity_to_cardinal(opacity);
    let _ = conn.change_property32(PropMode::REPLACE, wid, atom, AtomEnum::CARDINAL, &[value]);
    true
}

#[cfg(debug_assertions)]
fn clear_window_opacity(conn: &impl Connection, wid: u32) -> bool {
    let Some(atom) = intern(conn, b"_NET_WM_WINDOW_OPACITY") else {
        return false;
    };
    let _ = conn.delete_property(wid, atom);
    true
}

#[cfg(debug_assertions)]
fn compositor_running(conn: &impl Connection, screen_num: usize) -> bool {
    let selection = format!("_NET_WM_CM_S{screen_num}");
    let Some(atom) = intern(conn, selection.as_bytes()) else {
        return false;
    };
    let Ok(cookie) = conn.get_selection_owner(atom) else {
        return false;
    };
    cookie
        .reply()
        .map(|reply| reply.owner != 0)
        .unwrap_or(false)
}

#[cfg(debug_assertions)]
fn set_input_passthrough(conn: &impl Connection, wid: u32, passthrough: bool) -> bool {
    let full_rect;
    let rectangles: &[Rectangle] = if passthrough {
        &[]
    } else {
        let Ok(cookie) = conn.get_geometry(wid) else {
            return false;
        };
        let Ok(geometry) = cookie.reply() else {
            return false;
        };
        full_rect = [Rectangle {
            x: 0,
            y: 0,
            width: geometry.width,
            height: geometry.height,
        }];
        &full_rect
    };
    shape::rectangles(
        conn,
        shape::SO::SET,
        shape::SK::INPUT,
        ClipOrdering::UNSORTED,
        wid,
        0,
        0,
        rectangles,
    )
    .is_ok()
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

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::{normalize_opacity, opacity_to_cardinal};

    #[test]
    fn normalize_opacity_clamps_and_discards_invalid_values() {
        let cases = [
            (None, 0.0),
            (Some(-0.25), 0.0),
            (Some(0.7), 0.7),
            (Some(1.25), 1.0),
            (Some(f32::NAN), 0.0),
            (Some(f32::INFINITY), 0.0),
        ];
        for (input, expected) in cases {
            assert_eq!(normalize_opacity(input), expected);
        }
    }

    #[test]
    fn opacity_to_cardinal_maps_endpoints() {
        assert_eq!(opacity_to_cardinal(0.0), 0);
        assert_eq!(opacity_to_cardinal(1.0), u32::MAX);
    }
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
