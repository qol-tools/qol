use super::{DiscoveryError, WindowDiscovery, WindowInfo};
use qol_app_icon::RgbaImage;
use std::collections::HashMap;
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

#[derive(Default)]
struct IconCache {
    by_window: HashMap<u32, RgbaImage>,
    by_app: HashMap<String, RgbaImage>,
}

impl IconCache {
    fn get(&self, window_id: u32, app_name: &str) -> Option<RgbaImage> {
        if !app_name.is_empty() {
            return self.by_app.get(app_name).cloned();
        }
        self.by_window.get(&window_id).cloned()
    }

    fn store(&mut self, window_id: u32, app_name: &str, icon: RgbaImage) {
        self.by_window.insert(window_id, icon.clone());
        if !app_name.is_empty() {
            self.by_app.entry(app_name.to_string()).or_insert(icon);
        }
    }
}

fn discovery_session() -> &'static Mutex<Option<DiscoverySession>> {
    static SESSION: OnceLock<Mutex<Option<DiscoverySession>>> = OnceLock::new();
    SESSION.get_or_init(|| Mutex::new(connect_session()))
}

fn icon_cache() -> &'static Mutex<IconCache> {
    static CACHE: OnceLock<Mutex<IconCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(IconCache::default()))
}

fn connect_session() -> Option<DiscoverySession> {
    let (conn, screen_num) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots[screen_num].root;
    let atoms = intern_atoms(&conn);
    Some(DiscoverySession { conn, root, atoms })
}

fn focused_window_id(ids: &[u32], focused: &[bool], active: Option<u32>) -> Option<u32> {
    focused
        .iter()
        .position(|&f| f)
        .map(|i| ids[i])
        .or_else(|| active.filter(|a| ids.contains(a)))
}

fn next_mru(prev: &[u32], live_ids: &[u32], focused: Option<u32>) -> Vec<u32> {
    let mut mru: Vec<u32> = prev.to_vec();
    if let Some(f) = focused {
        mru.retain(|&x| x != f);
        mru.insert(0, f);
    }
    mru.retain(|x| live_ids.contains(x));
    mru
}

fn mru_order(ids: &[u32], above: &[bool], mru: &[u32]) -> Vec<usize> {
    let n = ids.len();
    let mut used = vec![false; n];
    let mut order = Vec::with_capacity(n);
    for &mid in mru {
        if let Some(i) = (0..n).find(|&i| !used[i] && ids[i] == mid) {
            used[i] = true;
            order.push(i);
        }
    }
    for i in 0..n {
        if !used[i] && !above[i] {
            used[i] = true;
            order.push(i);
        }
    }
    for (i, &was_used) in used.iter().enumerate() {
        if !was_used {
            order.push(i);
        }
    }
    order
}

fn mru_state() -> &'static Mutex<Vec<u32>> {
    static MRU: OnceLock<Mutex<Vec<u32>>> = OnceLock::new();
    MRU.get_or_init(|| Mutex::new(Vec::new()))
}

fn order_picker(
    windows: &mut Vec<WindowInfo>,
    above: &[bool],
    focused: &[bool],
    active: Option<u32>,
) {
    if windows.len() <= 1 {
        return;
    }
    let ids: Vec<u32> = windows.iter().map(|w| w.id).collect();
    let focused_id = focused_window_id(&ids, focused, active);
    let mru = {
        let mut guard = mru_state().lock().unwrap_or_else(|e| e.into_inner());
        *guard = next_mru(&guard, &ids, focused_id);
        guard.clone()
    };
    let order = mru_order(&ids, above, &mru);
    let mut slots: Vec<Option<WindowInfo>> = windows.drain(..).map(Some).collect();
    *windows = order.into_iter().filter_map(|i| slots[i].take()).collect();
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
    "_NET_WM_STATE_ABOVE",
    "_NET_WM_STATE_FOCUSED",
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
    let (mut windows, above, focused) =
        collect_window_info(&session.conn, session.root, &filtered, &session.atoms);
    let active = read_active_window(&session.conn, session.root, &session.atoms);
    order_picker(&mut windows, &above, &focused, active);

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
    state: Vec<Option<GetPropertyReply>>,
    geom: Vec<Option<WindowGeometry>>,
}

fn collect_window_info(
    conn: &impl Connection,
    root: u32,
    ids: &[u32],
    atoms: &AtomMap,
) -> (Vec<WindowInfo>, Vec<bool>, Vec<bool>) {
    let hidden_atom = atoms.get("_NET_WM_STATE_HIDDEN").copied().unwrap_or(0);
    let above_atom = atoms.get("_NET_WM_STATE_ABOVE").copied().unwrap_or(0);
    let focused_atom = atoms.get("_NET_WM_STATE_FOCUSED").copied().unwrap_or(0);
    let mut props = pipeline_and_resolve(conn, root, ids, atoms);
    let mut windows = Vec::with_capacity(ids.len());
    let mut above = Vec::with_capacity(ids.len());
    let mut focused = Vec::with_capacity(ids.len());

    for (i, &id) in ids.iter().enumerate().rev() {
        let Some((info, is_above, is_focused)) =
            build_window_info(id, i, &mut props, hidden_atom, above_atom, focused_atom)
        else {
            continue;
        };
        windows.push(info);
        above.push(is_above);
        focused.push(is_focused);
    }
    hydrate_icons(conn, atoms, &mut windows);
    (windows, above, focused)
}

fn pipeline_and_resolve(
    conn: &impl Connection,
    root: u32,
    ids: &[u32],
    atoms: &AtomMap,
) -> ResolvedProps {
    let state_atom = atoms.get("_NET_WM_STATE").copied();
    let net_name_atom = atoms.get("_NET_WM_NAME").copied();
    let wm_class_atom = atoms.get("WM_CLASS").copied().unwrap_or(0);

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
        geom: {
            let roots: Vec<_> = ids
                .iter()
                .map(|&id| conn.translate_coordinates(id, root, 0, 0).ok())
                .collect();
            let geometries: Vec<_> = ids.iter().map(|&id| conn.get_geometry(id).ok()).collect();
            geometries
                .into_iter()
                .zip(roots)
                .map(|(geom, root)| {
                    let geom = geom.and_then(|c| c.reply().ok())?;
                    let root = root.and_then(|c| c.reply().ok());
                    Some(WindowGeometry::from_replies(&geom, root.as_ref()))
                })
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
    above_atom: u32,
    focused_atom: u32,
) -> Option<(WindowInfo, bool, bool)> {
    let title = resolve_title(idx, props);
    if title.is_empty() || title == "Desktop" {
        return None;
    }
    let app_name = resolve_app_name(idx, props);
    let (is_minimized, is_above, is_focused) =
        resolve_states(idx, props, hidden_atom, above_atom, focused_atom);
    Some((
        WindowInfo {
            id,
            title,
            app_name,
            preview_path: None,
            icon: None,
            x: props.geom[idx].as_ref().map_or(0.0, |r| r.x),
            y: props.geom[idx].as_ref().map_or(0.0, |r| r.y),
            width: props.geom[idx].as_ref().map_or(0.0, |r| r.width),
            height: props.geom[idx].as_ref().map_or(0.0, |r| r.height),
            is_minimized,
        },
        is_above,
        is_focused,
    ))
}

#[derive(Clone, Copy)]
struct WindowGeometry {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}

impl WindowGeometry {
    fn from_replies(
        geom: &x11rb::protocol::xproto::GetGeometryReply,
        root: Option<&x11rb::protocol::xproto::TranslateCoordinatesReply>,
    ) -> Self {
        Self::from_parts(
            geom.x,
            geom.y,
            geom.width,
            geom.height,
            root.map(|r| (r.dst_x, r.dst_y)),
        )
    }

    fn from_parts(
        local_x: i16,
        local_y: i16,
        width: u16,
        height: u16,
        root: Option<(i16, i16)>,
    ) -> Self {
        let (x, y) = root.unwrap_or((local_x, local_y));
        Self {
            x: x as f32,
            y: y as f32,
            width: width as f32,
            height: height as f32,
        }
    }
}

fn hydrate_icons(conn: &impl Connection, atoms: &AtomMap, windows: &mut [WindowInfo]) {
    let wm_icon_atom = atoms.get("_NET_WM_ICON").copied().unwrap_or(0);
    if wm_icon_atom == 0 || windows.is_empty() {
        return;
    }

    let missing = fill_cached_icons(windows);
    if missing.is_empty() {
        return;
    }

    let replies = batch_icon_props(conn, &missing, wm_icon_atom);
    remember_fetched_icons(windows, missing, replies);
}

fn fill_cached_icons(windows: &mut [WindowInfo]) -> Vec<(usize, u32)> {
    let Ok(cache) = icon_cache().lock() else {
        return windows
            .iter()
            .enumerate()
            .map(|(idx, window)| (idx, window.id))
            .collect();
    };
    let mut missing = Vec::new();
    for (idx, window) in windows.iter_mut().enumerate() {
        if let Some(icon) = cache.get(window.id, &window.app_name) {
            window.icon = Some(icon);
        } else {
            missing.push((idx, window.id));
        }
    }
    missing
}

fn batch_icon_props(
    conn: &impl Connection,
    missing: &[(usize, u32)],
    wm_icon_atom: u32,
) -> Vec<Option<GetPropertyReply>> {
    let cookies: Vec<_> = missing
        .iter()
        .map(|&(_, id)| {
            conn.get_property(false, id, wm_icon_atom, AtomEnum::CARDINAL, 0, 65536)
                .ok()
        })
        .collect();
    cookies
        .into_iter()
        .map(|cookie| cookie.and_then(|cookie| cookie.reply().ok()))
        .collect()
}

fn remember_fetched_icons(
    windows: &mut [WindowInfo],
    missing: Vec<(usize, u32)>,
    replies: Vec<Option<GetPropertyReply>>,
) {
    let Ok(mut cache) = icon_cache().lock() else {
        for ((idx, _), reply) in missing.into_iter().zip(replies) {
            windows[idx].icon = reply.and_then(|reply| extract_x11_icon(&reply));
        }
        return;
    };

    for ((idx, window_id), reply) in missing.into_iter().zip(replies) {
        let Some(icon) = reply.and_then(|reply| extract_x11_icon(&reply)) else {
            continue;
        };
        cache.store(window_id, &windows[idx].app_name, icon.clone());
        windows[idx].icon = Some(icon);
    }
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

fn resolve_states(
    idx: usize,
    props: &mut ResolvedProps,
    hidden_atom: u32,
    above_atom: u32,
    focused_atom: u32,
) -> (bool, bool, bool) {
    let atoms: Vec<u32> = props.state[idx]
        .take()
        .and_then(|r| r.value32().map(|it| it.collect()))
        .unwrap_or_default();
    let is_minimized = atoms.contains(&hidden_atom);
    let is_above = above_atom != 0 && atoms.contains(&above_atom);
    let is_focused = focused_atom != 0 && atoms.contains(&focused_atom);
    (is_minimized, is_above, is_focused)
}

fn extract_x11_icon(reply: &GetPropertyReply) -> Option<RgbaImage> {
    let values: Vec<u32> = reply.value32()?.collect();
    if values.len() < 2 {
        return None;
    }
    let (data_start, src_w, src_h) = pick_best_icon(&values)?;
    Some(argb_to_bgra(&values[data_start..], src_w, src_h, 32))
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn icon(byte: u8) -> RgbaImage {
        RgbaImage {
            data: vec![byte; 4],
            width: 1,
            height: 1,
        }
    }

    fn assert_icon(actual: Option<RgbaImage>, expected_byte: u8) {
        let actual = actual.expect("expected cached icon");
        assert_eq!(actual.data, vec![expected_byte; 4]);
        assert_eq!((actual.width, actual.height), (1, 1));
    }

    #[test]
    fn icon_cache_reuses_app_icon_for_new_window() {
        let mut cache = IconCache::default();
        cache.store(1, "Terminal", icon(7));

        assert_icon(cache.get(2, "Terminal"), 7);
    }

    #[test]
    fn icon_cache_ignores_reused_window_id_for_different_app() {
        let mut cache = IconCache::default();
        cache.store(1, "Terminal", icon(7));

        assert!(cache.get(1, "Browser").is_none());
    }

    #[test]
    fn icon_cache_uses_window_id_when_app_name_is_empty() {
        let mut cache = IconCache::default();
        cache.store(1, "", icon(9));

        assert_icon(cache.get(1, ""), 9);
    }

    #[test]
    fn root_translation_wins_over_parent_relative_geometry() {
        let geom = WindowGeometry::from_parts(0, 0, 1920, 1080, Some((2560, 0)));

        assert_eq!(geom.x, 2560.0);
        assert_eq!(geom.y, 0.0);
        assert_eq!(geom.width, 1920.0);
        assert_eq!(geom.height, 1080.0);
    }

    type NextMruCase = (
        &'static str,
        &'static [u32],
        &'static [u32],
        Option<u32>,
        &'static [u32],
    );

    #[test]
    fn next_mru_table() {
        let cases: &[NextMruCase] = &[
            (
                "focused promotes front, dedup keeps tail",
                &[10, 20],
                &[10, 20, 30],
                Some(30),
                &[30, 10, 20],
            ),
            ("no focus, dead ids pruned", &[10, 20], &[20], None, &[20]),
            (
                "focused moves from tail to front",
                &[20, 10],
                &[10, 20],
                Some(10),
                &[10, 20],
            ),
            (
                "empty prev, focused seeds front",
                &[],
                &[10, 20],
                Some(10),
                &[10],
            ),
            (
                "no focus, no death, unchanged",
                &[10, 20],
                &[10, 20],
                None,
                &[10, 20],
            ),
        ];
        for (label, prev, live, focused, expected) in cases {
            let got = next_mru(prev, live, *focused);
            assert_eq!(got.as_slice(), *expected, "case: {label}");
        }
    }

    type MruOrderCase = (
        &'static str,
        &'static [u32],
        &'static [bool],
        &'static [u32],
        &'static [usize],
    );

    #[test]
    fn mru_order_table() {
        const PANEL: u32 = 0x4c00004;
        const EDITOR: u32 = 0x660000b;
        const FIREFOX: u32 = 0x6c00004;
        let cases: &[MruOrderCase] = &[
            (
                "panel just-left, editor focused",
                &[PANEL, EDITOR, FIREFOX],
                &[true, false, false],
                &[EDITOR, PANEL],
                &[1, 0, 2],
            ),
            (
                "panel focused, panel slot 0",
                &[PANEL, EDITOR, FIREFOX],
                &[true, false, false],
                &[PANEL],
                &[0, 1, 2],
            ),
            (
                "empty mru, panel sinks last",
                &[PANEL, EDITOR, FIREFOX],
                &[true, false, false],
                &[],
                &[1, 2, 0],
            ),
            (
                "mru has dead id, live ones still order",
                &[10, 20],
                &[false, false],
                &[999, 20],
                &[1, 0],
            ),
            (
                "all normal, mru orders them",
                &[10, 20, 30],
                &[false, false, false],
                &[30, 10],
                &[2, 0, 1],
            ),
        ];
        for (label, ids, above, mru, expected) in cases {
            let got = mru_order(ids, above, mru);
            assert_eq!(got.as_slice(), *expected, "case: {label}");
        }
    }

    #[test]
    #[ignore = "connects to the live X server; run with --ignored --nocapture"]
    fn live_picker_order() {
        let windows = super::get_open_windows();
        eprintln!("[live] {} windows", windows.len());
        for (i, w) in windows.iter().enumerate() {
            eprintln!("[live] slot {i}: {:?} id={:#x}", w.title, w.id);
        }
        assert!(!windows.is_empty(), "expected at least one live window");
    }

    #[test]
    fn geometry_falls_back_to_local_position_without_translation() {
        let geom = WindowGeometry::from_parts(12, 34, 800, 600, None);

        assert_eq!(geom.x, 12.0);
        assert_eq!(geom.y, 34.0);
        assert_eq!(geom.width, 800.0);
        assert_eq!(geom.height, 600.0);
    }
}
