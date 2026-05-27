use super::{DiscoveryError, WindowDiscovery, WindowInfo};
use qol_app_icon::RgbaImage;
use std::sync::{Mutex, OnceLock};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt as _;
use x11rb::protocol::xproto::*;
use x11rb::rust_connection::RustConnection;

pub struct Platform;

impl WindowDiscovery for Platform {
    fn visible_windows(&self, include_minimized: bool) -> Result<Vec<WindowInfo>, DiscoveryError> {
        let mut windows = get_open_windows();
        if !include_minimized {
            windows.retain(|w| !w.is_minimized);
        }
        Ok(windows)
    }
}

struct DiscoverySession {
    conn: RustConnection,
    root: u32,
    atoms: AtomMap,
}

fn discovery_session() -> &'static Mutex<Option<DiscoverySession>> {
    static SESSION: OnceLock<Mutex<Option<DiscoverySession>>> = OnceLock::new();
    SESSION.get_or_init(|| Mutex::new(connect_session()))
}

fn connect_session() -> Option<DiscoverySession> {
    let (conn, screen_num) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots[screen_num].root;
    let atoms = intern_atoms(&conn);
    Some(DiscoverySession { conn, root, atoms })
}

fn promote_active_info(
    conn: &impl Connection,
    root: u32,
    atoms: &AtomMap,
    windows: &mut Vec<WindowInfo>,
) {
    let Some(active) = read_active_window(conn, root, atoms) else {
        return;
    };
    let Some(pos) = windows.iter().position(|w| w.id == active) else {
        return;
    };
    if pos > 0 {
        let w = windows.remove(pos);
        windows.insert(0, w);
    }
}

fn read_active_window(conn: &impl Connection, root: u32, atoms: &AtomMap) -> Option<u32> {
    let atom = atoms.get("_NET_ACTIVE_WINDOW").copied()?;
    let reply = conn
        .get_property(false, root, atom, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    reply.value32().and_then(|mut iter| {
        let id = iter.next()?;
        if id == 0 {
            None
        } else {
            Some(id)
        }
    })
}

type AtomMap = std::collections::HashMap<&'static str, u32>;

const ATOM_NAMES: &[&str] = &[
    "_NET_CLIENT_LIST",
    "_NET_CLIENT_LIST_STACKING",
    "_NET_WM_NAME",
    "UTF8_STRING",
    "_NET_WM_WINDOW_TYPE",
    "_NET_WM_WINDOW_TYPE_NORMAL",
    "_NET_WM_STATE",
    "_NET_WM_STATE_HIDDEN",
    "WM_CLASS",
    "_NET_WM_ICON",
    "_NET_ACTIVE_WINDOW",
];

pub fn get_open_windows() -> Vec<WindowInfo> {
    let Ok(mut guard) = discovery_session().lock() else {
        return Vec::new();
    };
    let session = match &*guard {
        Some(s) => s,
        None => {
            *guard = connect_session();
            match &*guard {
                Some(s) => s,
                None => return Vec::new(),
            }
        }
    };
    let result = get_open_windows_with(session);
    if result.is_empty() {
        *guard = connect_session();
    }
    result
}

fn get_open_windows_with(session: &DiscoverySession) -> Vec<WindowInfo> {
    let ids = fetch_window_ids(&session.conn, session.root, &session.atoms);
    if ids.is_empty() {
        return Vec::new();
    }
    let filtered = filter_normal_windows(&session.conn, &ids, &session.atoms);
    let mut windows = collect_window_info(&session.conn, &filtered, &session.atoms);
    promote_active_info(&session.conn, session.root, &session.atoms, &mut windows);

    #[cfg(debug_assertions)]
    eprintln!("[x11] get_open_windows total results: {}", windows.len());

    windows
}

fn intern_atoms(conn: &impl Connection) -> AtomMap {
    let cookies: Vec<_> = ATOM_NAMES
        .iter()
        .map(|name| conn.intern_atom(false, name.as_bytes()).ok())
        .collect();
    let mut map = AtomMap::new();
    for (i, cookie) in cookies.into_iter().enumerate() {
        let Some(reply) = cookie.and_then(|c| c.reply().ok()) else {
            continue;
        };
        map.insert(ATOM_NAMES[i], reply.atom);
    }
    map
}

fn fetch_window_ids(conn: &impl Connection, root: u32, atoms: &AtomMap) -> Vec<u32> {
    let list_atom = atoms
        .get("_NET_CLIENT_LIST_STACKING")
        .or_else(|| atoms.get("_NET_CLIENT_LIST"))
        .copied()
        .unwrap_or(0);
    if list_atom == 0 {
        return Vec::new();
    }
    let prop = conn
        .get_property(false, root, list_atom, AtomEnum::WINDOW, 0, 1024)
        .ok()
        .and_then(|c| c.reply().ok());
    let Some(prop) = prop else {
        return Vec::new();
    };
    let Some(value32) = prop.value32() else {
        return Vec::new();
    };
    value32.collect()
}

fn filter_normal_windows(conn: &impl Connection, ids: &[u32], atoms: &AtomMap) -> Vec<u32> {
    let type_atom = atoms.get("_NET_WM_WINDOW_TYPE").copied();
    let normal_atom = atoms
        .get("_NET_WM_WINDOW_TYPE_NORMAL")
        .copied()
        .unwrap_or(0);

    let type_cookies: Vec<_> = ids
        .iter()
        .map(|&id| {
            type_atom.and_then(|ta| conn.get_property(false, id, ta, AtomEnum::ATOM, 0, 10).ok())
        })
        .collect();

    let mut filtered = Vec::with_capacity(ids.len());
    for (i, cookie) in type_cookies.into_iter().enumerate() {
        let reply = cookie.and_then(|c| c.reply().ok());
        if is_normal_window_type(reply.as_ref(), normal_atom) {
            filtered.push(ids[i]);
        }
    }
    filtered
}

fn is_normal_window_type(reply: Option<&GetPropertyReply>, normal_atom: u32) -> bool {
    let Some(reply) = reply else {
        return true;
    };
    let Some(types) = reply.value32() else {
        return true;
    };
    let types: Vec<u32> = types.collect();
    if types.is_empty() {
        return true;
    }
    types.contains(&normal_atom)
}

struct ResolvedProps {
    net_name: Vec<Option<GetPropertyReply>>,
    wm_name: Vec<Option<GetPropertyReply>>,
    wm_class: Vec<Option<GetPropertyReply>>,
    icon: Vec<Option<GetPropertyReply>>,
    state: Vec<Option<GetPropertyReply>>,
    geom: Vec<Option<x11rb::protocol::xproto::GetGeometryReply>>,
}

fn collect_window_info(conn: &impl Connection, ids: &[u32], atoms: &AtomMap) -> Vec<WindowInfo> {
    let hidden_atom = atoms.get("_NET_WM_STATE_HIDDEN").copied().unwrap_or(0);
    let mut props = pipeline_and_resolve(conn, ids, atoms);
    let mut windows = Vec::with_capacity(ids.len());

    for (i, &id) in ids.iter().enumerate().rev() {
        let info = build_window_info(id, i, &mut props, hidden_atom);
        let Some(info) = info else { continue };
        windows.push(info);
    }
    windows
}

fn pipeline_and_resolve(conn: &impl Connection, ids: &[u32], atoms: &AtomMap) -> ResolvedProps {
    let state_atom = atoms.get("_NET_WM_STATE").copied();
    let net_name_atom = atoms.get("_NET_WM_NAME").copied();
    let wm_class_atom = atoms.get("WM_CLASS").copied().unwrap_or(0);
    let wm_icon_atom = atoms.get("_NET_WM_ICON").copied().unwrap_or(0);

    ResolvedProps {
        state: batch_prop(conn, ids, |c, id| {
            state_atom.and_then(|a| c.get_property(false, id, a, AtomEnum::ATOM, 0, 64).ok())
        }),
        net_name: batch_prop(conn, ids, |c, id| {
            net_name_atom.and_then(|a| c.get_property(false, id, a, AtomEnum::ANY, 0, 1024).ok())
        }),
        wm_name: batch_prop(conn, ids, |c, id| {
            c.get_property(false, id, AtomEnum::WM_NAME, AtomEnum::ANY, 0, 1024)
                .ok()
        }),
        wm_class: batch_prop(conn, ids, |c, id| {
            c.get_property(false, id, wm_class_atom, AtomEnum::STRING, 0, 1024)
                .ok()
        }),
        icon: batch_prop(conn, ids, |c, id| {
            if wm_icon_atom != 0 {
                c.get_property(false, id, wm_icon_atom, AtomEnum::CARDINAL, 0, 65536)
                    .ok()
            } else {
                None
            }
        }),
        geom: {
            let cookies: Vec<_> = ids.iter().map(|&id| conn.get_geometry(id).ok()).collect();
            cookies
                .into_iter()
                .map(|c| c.and_then(|c| c.reply().ok()))
                .collect()
        },
    }
}

fn batch_prop<C: Connection>(
    conn: &C,
    ids: &[u32],
    fire: impl Fn(&C, u32) -> Option<x11rb::cookie::Cookie<'_, C, GetPropertyReply>>,
) -> Vec<Option<GetPropertyReply>> {
    let cookies: Vec<_> = ids.iter().map(|&id| fire(conn, id)).collect();
    cookies
        .into_iter()
        .map(|c| c.and_then(|c| c.reply().ok()))
        .collect()
}

fn build_window_info(
    id: u32,
    idx: usize,
    props: &mut ResolvedProps,
    hidden_atom: u32,
) -> Option<WindowInfo> {
    let title = resolve_title(idx, props);
    if title.is_empty() || title == "Desktop" {
        return None;
    }
    Some(WindowInfo {
        id,
        title,
        app_name: resolve_app_name(idx, props),
        preview_path: None,
        icon: props.icon[idx].take().and_then(|r| extract_x11_icon(&r)),
        x: props.geom[idx].as_ref().map_or(0.0, |r| r.x as f32),
        y: props.geom[idx].as_ref().map_or(0.0, |r| r.y as f32),
        width: props.geom[idx].as_ref().map_or(0.0, |r| r.width as f32),
        height: props.geom[idx].as_ref().map_or(0.0, |r| r.height as f32),
        is_minimized: resolve_minimized(idx, props, hidden_atom),
    })
}

fn resolve_title(idx: usize, props: &mut ResolvedProps) -> String {
    if let Some(reply) = props.net_name[idx].take() {
        let title = String::from_utf8_lossy(&reply.value).into_owned();
        if !title.is_empty() {
            return title;
        }
    }
    props.wm_name[idx]
        .take()
        .map(|r| String::from_utf8_lossy(&r.value).into_owned())
        .unwrap_or_default()
}

fn resolve_app_name(idx: usize, props: &mut ResolvedProps) -> String {
    let Some(reply) = props.wm_class[idx].take() else {
        return String::new();
    };
    let parts: Vec<&str> = std::str::from_utf8(&reply.value)
        .unwrap_or("")
        .split('\0')
        .filter(|s| !s.is_empty())
        .collect();
    if parts.len() >= 2 {
        return parts[1].to_string();
    }
    parts.first().map(|s| s.to_string()).unwrap_or_default()
}

fn resolve_minimized(idx: usize, props: &mut ResolvedProps, hidden_atom: u32) -> bool {
    props.state[idx]
        .take()
        .and_then(|r| {
            r.value32()
                .map(|atoms| atoms.into_iter().any(|a| a == hidden_atom))
        })
        .unwrap_or(false)
}

fn extract_x11_icon(reply: &GetPropertyReply) -> Option<RgbaImage> {
    let values: Vec<u32> = reply.value32()?.collect();
    if values.len() < 2 {
        return None;
    }
    let (data_start, src_w, src_h) = pick_best_icon(&values)?;
    Some(argb_to_bgra(&values[data_start..], src_w, src_h, 32))
}

/// Walk _NET_WM_ICON entries and pick the best icon ≤ 48px, or smallest available.
/// Returns (data_start_offset, width, height).
fn pick_best_icon(values: &[u32]) -> Option<(usize, usize, usize)> {
    let mut offset = 0;
    let mut best: Option<(usize, usize, usize)> = None;
    while offset + 2 < values.len() {
        let w = values[offset] as usize;
        let h = values[offset + 1] as usize;
        let pixel_count = w.checked_mul(h).unwrap_or(0);
        if w == 0 || h == 0 || offset + 2 + pixel_count > values.len() {
            break;
        }
        best = Some(better_icon(best, offset + 2, w, h));
        offset += 2 + pixel_count;
    }
    best
}

fn better_icon(
    current: Option<(usize, usize, usize)>,
    data_start: usize,
    w: usize,
    h: usize,
) -> (usize, usize, usize) {
    let Some((_, bw, bh)) = current else {
        return (data_start, w, h);
    };
    if w <= 48 && h <= 48 && w * h > bw * bh {
        return (data_start, w, h);
    }
    if bw > 48 && w < bw {
        return (data_start, w, h);
    }
    current.unwrap()
}

/// Nearest-neighbor resize from ARGB u32 pixels to BGRA byte buffer.
fn argb_to_bgra(pixels: &[u32], src_w: usize, src_h: usize, target: usize) -> RgbaImage {
    let mut bgra = vec![0u8; target * target * 4];
    for y in 0..target {
        let src_y = (y * src_h) / target;
        for x in 0..target {
            let src_x = (x * src_w) / target;
            let argb = pixels[src_y * src_w + src_x];
            let dst = (y * target + x) * 4;
            bgra[dst] = (argb & 0xff) as u8;
            bgra[dst + 1] = ((argb >> 8) & 0xff) as u8;
            bgra[dst + 2] = ((argb >> 16) & 0xff) as u8;
            bgra[dst + 3] = ((argb >> 24) & 0xff) as u8;
        }
    }
    RgbaImage {
        data: bgra,
        width: target,
        height: target,
    }
}
