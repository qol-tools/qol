use x11rb::connection::Connection;
use x11rb::properties::WmHints;
use x11rb::protocol::shape;
use x11rb::protocol::xproto::*;
use x11rb::protocol::Event;
use x11rb::wrapper::ConnectionExt as _;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::PopupPresentation;

pub struct Platform;

impl PopupPresentation for Platform {
    fn present_topmost(title: &str) {
        force_composite_below(composite_owner(title));
        make_override_redirect(title);
    }

    fn restore_composite(title: &str) {
        restore_composite(composite_owner(title));
    }
}

fn composite_owner(title: &str) -> &str {
    title.split('@').next().unwrap_or(title)
}

static GHOST_ALPHA: AtomicU32 = AtomicU32::new(0);

static OPACITY_CACHE: Mutex<BTreeMap<String, (u32, Option<u32>)>> = Mutex::new(BTreeMap::new());

fn cached_card(title: &str) -> Option<u32> {
    OPACITY_CACHE
        .lock()
        .ok()
        .and_then(|cache| cache.get(title).and_then(|entry| entry.1))
}

fn store_card(title: &str, wid: u32, card: Option<u32>) {
    if let Ok(mut cache) = OPACITY_CACHE.lock() {
        cache.insert(title.to_string(), (wid, card));
    }
}

fn cached_valid_wid(
    conn: &impl Connection,
    name_atom: u32,
    utf8_atom: u32,
    title: &str,
) -> Option<u32> {
    let wid = OPACITY_CACHE.lock().ok()?.get(title).map(|entry| entry.0)?;
    let matches = window_title_matches(conn, wid, name_atom, utf8_atom, title)
        || window_title_matches(
            conn,
            wid,
            AtomEnum::WM_NAME.into(),
            AtomEnum::ANY.into(),
            title,
        );
    matches.then_some(wid)
}

fn resolve_window(
    conn: &impl Connection,
    root: u32,
    list_atom: u32,
    name_atom: u32,
    utf8_atom: u32,
    title: &str,
) -> Option<u32> {
    if let Some(wid) = cached_valid_wid(conn, name_atom, utf8_atom, title) {
        return Some(wid);
    }
    let wid = find_window_by_title(conn, root, list_atom, name_atom, utf8_atom, title)?;
    store_card(title, wid, None);
    Some(wid)
}

fn connect_with_atoms() -> Option<(impl Connection, usize, u32, u32, u32, u32)> {
    let (conn, screen_num) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots[screen_num].root;
    let list_atom = intern(&conn, b"_NET_CLIENT_LIST")?;
    let name_atom = intern(&conn, b"_NET_WM_NAME")?;
    let utf8_atom = intern(&conn, b"UTF8_STRING")?;
    Some((conn, screen_num, root, list_atom, name_atom, utf8_atom))
}

#[derive(Clone)]
pub struct WindowGeometrySession {
    conn: Arc<x11rb::rust_connection::RustConnection>,
    root: u32,
    wid: u32,
}

pub fn window_geometry_session(title: &str) -> Option<WindowGeometrySession> {
    let (conn, screen_num) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots[screen_num].root;
    let list_atom = intern(&conn, b"_NET_CLIENT_LIST")?;
    let name_atom = intern(&conn, b"_NET_WM_NAME")?;
    let utf8_atom = intern(&conn, b"UTF8_STRING")?;
    let wid = resolve_window(&conn, root, list_atom, name_atom, utf8_atom, title)?;
    Some(WindowGeometrySession {
        conn: Arc::new(conn),
        root,
        wid,
    })
}

impl WindowGeometrySession {
    pub fn set_bounds(&self, x: i32, y: i32, width: u32, height: u32) {
        let aux = ConfigureWindowAux::new()
            .x(x)
            .y(y)
            .width(width.max(1))
            .height(height.max(1));
        let _ = self.conn.configure_window(self.wid, &aux);
        let _ = self.conn.flush();
    }

    pub fn set_position(&self, x: i32, y: i32) {
        let aux = ConfigureWindowAux::new().x(x).y(y);
        let _ = self.conn.configure_window(self.wid, &aux);
        let _ = self.conn.flush();
    }

    pub fn pointer_root(&self) -> Option<(i32, i32)> {
        let reply = self.conn.query_pointer(self.root).ok()?.reply().ok()?;
        Some((i32::from(reply.root_x), i32::from(reply.root_y)))
    }

    pub fn bounds(&self) -> Option<(i32, i32, u32, u32)> {
        let geometry = self.conn.get_geometry(self.wid).ok()?.reply().ok()?;
        let coords = self
            .conn
            .translate_coordinates(self.wid, self.root, 0, 0)
            .ok()?
            .reply()
            .ok()?;
        Some((
            i32::from(coords.dst_x),
            i32::from(coords.dst_y),
            u32::from(geometry.width),
            u32::from(geometry.height),
        ))
    }

    pub fn anchor_content(&self, right: bool, bottom: bool) {
        let gravity = match (right, bottom) {
            (false, false) => Gravity::NORTH_WEST,
            (true, false) => Gravity::NORTH_EAST,
            (false, true) => Gravity::SOUTH_WEST,
            (true, true) => Gravity::SOUTH_EAST,
        };
        let attributes = ChangeWindowAttributesAux::new().bit_gravity(gravity);
        let _ = self.conn.change_window_attributes(self.wid, &attributes);
        let _ = self.conn.flush();
    }
}

pub fn window_position_by_title(title: &str) -> Option<(i32, i32)> {
    let (conn, _screen_num, root, list_atom, name_atom, utf8_atom) = connect_with_atoms()?;
    let wid = resolve_window(&conn, root, list_atom, name_atom, utf8_atom, title)?;
    let frame = top_level_frame(&conn, root, wid).unwrap_or(wid);
    let geometry = conn.get_geometry(frame).ok()?.reply().ok()?;
    Some((geometry.x as i32, geometry.y as i32))
}

pub fn reposition_window_by_title(title: &str, gpui_x: f64, gpui_y: f64) -> bool {
    let Some((conn, _screen_num, root, list_atom, name_atom, utf8_atom)) = connect_with_atoms()
    else {
        return false;
    };

    let Some(wid) = resolve_window(&conn, root, list_atom, name_atom, utf8_atom, title) else {
        return false;
    };
    move_window(&conn, root, wid, gpui_x as i32, gpui_y as i32)
}

fn set_window_bounds_by_title(
    title: &str,
    gpui_x: f64,
    gpui_y: f64,
    gpui_width: f64,
    gpui_height: f64,
) -> bool {
    let Some((conn, _screen_num, root, list_atom, name_atom, utf8_atom)) = connect_with_atoms()
    else {
        return false;
    };

    let Some(wid) = resolve_window(&conn, root, list_atom, name_atom, utf8_atom, title) else {
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

pub fn sync_window_layout(
    title: &str,
    window: &mut gpui::Window,
    origin: gpui::Point<gpui::Pixels>,
    size: gpui::Size<gpui::Pixels>,
) -> bool {
    let backing = window_backing_scale(title);
    crate::window::resize_or_sync_scale(window, size, backing);
    set_window_bounds_by_title(
        title,
        origin.x.to_f64(),
        origin.y.to_f64(),
        size.width.to_f64(),
        size.height.to_f64(),
    )
}

pub fn hide_window_by_title(title: &str) -> bool {
    hide_window_with_opacity(title, ghost_opacity())
}

pub fn hide_invisible(title: &str) -> bool {
    hide_window_with_opacity(title, 0.0)
}

pub fn park_window_by_title(title: &str) -> bool {
    let Some((conn, _screen_num, root, list_atom, name_atom, utf8_atom)) = connect_with_atoms()
    else {
        return false;
    };
    let Some(wid) = resolve_window(&conn, root, list_atom, name_atom, utf8_atom, title) else {
        return false;
    };
    let parked = conn
        .unmap_window(wid)
        .ok()
        .and_then(|cookie| cookie.check().ok())
        .is_some();
    let flushed = conn.flush().is_ok();
    store_card(title, wid, None);
    qol_runtime::probe!(
        "PARK_WIN",
        "title={title} wid={wid} unmap={parked} flush={flushed}"
    );
    parked && flushed
}

pub fn prepare_window_reveal_by_title(title: &str) -> bool {
    #[cfg(debug_assertions)]
    let reason = crate::popup_window::change_reason();
    let Some((conn, screen_num, root, list_atom, name_atom, utf8_atom)) = connect_with_atoms()
    else {
        return false;
    };
    let Some(wid) = resolve_window(&conn, root, list_atom, name_atom, utf8_atom, title) else {
        qol_runtime::probe!("PREPARE_WIN", "title={title} wid=NONE reason={reason}");
        return false;
    };
    if !compositor_running(&conn, screen_num) {
        qol_runtime::probe!(
            "PREPARE_WIN",
            "title={title} wid={wid} compositor=false reason={reason}"
        );
        return false;
    }
    let input_ok = set_input_passthrough(&conn, wid, true);
    let opacity_ok = set_window_opacity(&conn, wid, 0.0);
    let map_ok = conn
        .map_window(wid)
        .ok()
        .and_then(|cookie| cookie.check().ok())
        .is_some();
    add_window_state(&conn, root, wid);
    let flush_ok = conn.flush().is_ok();
    let prepared = input_ok && opacity_ok && map_ok && flush_ok;
    if prepared {
        store_card(title, wid, Some(opacity_to_cardinal(0.0)));
    }
    qol_runtime::probe!(
        "PREPARE_WIN",
        "title={title} wid={wid} compositor=true input_shape_ok={input_ok} opacity=0 opacity_ok={opacity_ok} map={map_ok} flush={flush_ok} prepared={prepared} reason={reason}"
    );
    prepared
}

pub fn configure_keepalive_window(title: &str) -> bool {
    let Some((conn, _screen_num, root, list_atom, name_atom, utf8_atom)) = connect_with_atoms()
    else {
        return false;
    };
    let Some(wid) = resolve_window(&conn, root, list_atom, name_atom, utf8_atom, title) else {
        return false;
    };

    let input_hint = set_keepalive_input_hint(&conn, wid);
    let input_shape = set_input_passthrough(&conn, wid, true);
    let unmapped = conn
        .unmap_window(wid)
        .ok()
        .and_then(|cookie| cookie.check().ok())
        .is_some();
    let flushed = conn.flush().is_ok();
    store_card(title, wid, None);

    let configured = input_hint && input_shape && unmapped && flushed;
    qol_runtime::probe!(
        "KEEPALIVE",
        "title={title} wid={wid} input_hint=false input_hint_applied={input_hint} input_passthrough={input_shape} map=unmapped unmap={unmapped} flush={flushed}"
    );
    configured
}

pub fn hide_windows_by_title_prefix(prefix: &str) -> usize {
    cached_titles_by_prefix(prefix)
        .iter()
        .filter(|title| hide_invisible(title))
        .count()
}

pub fn visible_windows_by_title_prefix(prefix: &str) -> usize {
    let entries = cached_windows_by_prefix(prefix);
    let Ok((conn, _screen_num)) = x11rb::connect(None) else {
        return entries.len();
    };
    entries
        .into_iter()
        .filter(|wid| window_is_visible(&conn, *wid))
        .count()
}

fn cached_titles_by_prefix(prefix: &str) -> Vec<String> {
    OPACITY_CACHE
        .lock()
        .map(|cache| {
            cache
                .keys()
                .filter(|title| title.starts_with(prefix))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn cached_windows_by_prefix(prefix: &str) -> Vec<u32> {
    OPACITY_CACHE
        .lock()
        .map(|cache| {
            cache
                .iter()
                .filter(|(title, _)| title.starts_with(prefix))
                .map(|(_, (wid, _))| *wid)
                .collect()
        })
        .unwrap_or_default()
}

pub fn hide_for_capture(title: &str, _window: &mut gpui::Window) -> bool {
    hide_invisible(title)
}

fn hide_window_with_opacity(title: &str, opacity: f32) -> bool {
    #[cfg(debug_assertions)]
    let reason = crate::popup_window::change_reason();
    let target = opacity_to_cardinal(opacity);
    let Some((conn, screen_num, root, list_atom, name_atom, utf8_atom)) = connect_with_atoms()
    else {
        return false;
    };
    let Some(wid) = resolve_window(&conn, root, list_atom, name_atom, utf8_atom, title) else {
        qol_runtime::probe!("HIDE_WIN", "title={title} wid=NONE reason={reason}");
        return false;
    };
    release_input_focus(&conn, root, wid);
    if cached_card(title) == Some(target) {
        return true;
    }
    if compositor_running(&conn, screen_num) && set_input_passthrough(&conn, wid, true) {
        let applied = set_window_opacity(&conn, wid, opacity);
        let _ = conn.flush();
        if applied {
            store_card(title, wid, Some(target));
        }
        qol_runtime::probe!(
            "HIDE_WIN",
            "title={title} wid={wid} path=compositor opacity={opacity} applied={applied} reason={reason}",
        );
        return true;
    }
    let _ = conn.unmap_window(wid);
    let _ = conn.flush();
    store_card(title, wid, None);
    qol_runtime::probe!(
        "HIDE_WIN",
        "title={title} wid={wid} path=unmap reason={reason}"
    );
    true
}

pub fn show_window_by_title(title: &str) -> bool {
    show_window_by_title_with_focus(title, true, WindowPresentation::Overlay)
}

pub fn show_window_passive_by_title(title: &str) -> bool {
    show_window_by_title_with_focus(title, false, WindowPresentation::Overlay)
}

pub fn show_normal_window_by_title(title: &str) -> bool {
    show_window_by_title_with_focus(title, true, WindowPresentation::Normal)
}

#[derive(Clone, Copy, Debug)]
enum WindowPresentation {
    Overlay,
    Normal,
}

fn show_window_by_title_with_focus(
    title: &str,
    focus: bool,
    presentation: WindowPresentation,
) -> bool {
    #[cfg(debug_assertions)]
    let reason = crate::popup_window::change_reason();
    let Some((conn, _screen_num, root, list_atom, name_atom, utf8_atom)) = connect_with_atoms()
    else {
        return false;
    };
    let Some(wid) = resolve_window(&conn, root, list_atom, name_atom, utf8_atom, title) else {
        qol_runtime::probe!("SHOW_WIN", "title={title} wid=NONE reason={reason}");
        return false;
    };
    let active_before = active_window(&conn, root);
    let before = show_window_state(&conn, root, wid, active_before);
    qol_runtime::probe!(
        "SHOW_WIN_STATE",
        "reason={reason} phase=before title={title} wid={wid} {before}"
    );
    let clear_ok = clear_window_opacity(&conn, wid);
    store_card(title, wid, None);
    let input_ok = set_input_passthrough(&conn, wid, !focus);
    let map_ok = conn
        .map_window(wid)
        .ok()
        .and_then(|cookie| cookie.check().ok())
        .is_some();
    let state_ok = match presentation {
        WindowPresentation::Overlay => {
            add_window_state(&conn, root, wid);
            true
        }
        WindowPresentation::Normal => {
            clear_window_type(&conn, wid) && clear_panel_window_state(&conn, root, wid)
        }
    };
    let stack = raise_window(&conn, root, wid);
    let (activate_ok, timestamp, focus_ok) = if focus {
        let (activate_ok, timestamp) = activate_window(&conn, root, wid);
        let focus_ok = take_input_focus(&conn, root, wid, active_before);
        (activate_ok, timestamp, focus_ok)
    } else {
        (true, 0, true)
    };
    let flush_ok = conn.flush().is_ok();
    let active_after = active_window(&conn, root);
    let after = show_window_state(&conn, root, wid, active_after);
    #[cfg(not(debug_assertions))]
    let _ = (
        &before,
        &clear_ok,
        &input_ok,
        &map_ok,
        &state_ok,
        &stack.frame,
        &stack.client,
        &stack.frame_ok,
        &activate_ok,
        &timestamp,
        &focus_ok,
        &flush_ok,
        &after,
    );
    qol_runtime::probe!(
        "SHOW_WIN_STATE",
        "reason={reason} phase=after title={title} wid={wid} presentation={presentation:?} frame={} clear_opacity={clear_ok} input_shape_ok={input_ok} map={map_ok} state={state_ok} stack_client={} stack_frame={} focus_requested={focus} activate={activate_ok} focus={focus_ok} timestamp={timestamp} flush={flush_ok} {after}",
        stack.frame,
        stack.client,
        stack.frame_ok,
    );
    qol_runtime::probe!(
        "SHOW_WIN",
        "title={title} wid={wid} presentation={presentation:?} cleared_opacity={clear_ok} state={state_ok} source=2 focus_requested={focus} timestamp={timestamp} requester_active=0 reason={reason}",
    );
    true
}

fn show_window_state(conn: &impl Connection, root: u32, wid: u32, active: Option<u32>) -> String {
    let opacity = window_opacity(conn, wid)
        .map(|value| format!("{value:.2}"))
        .unwrap_or_else(|| "unset".to_string());
    let (x, y, width, height) = absolute_geometry(conn, root, wid)
        .map(|(x, y, width, height)| (x, y, width.to_string(), height.to_string()))
        .unwrap_or_else(|| (i32::MIN, i32::MIN, "?".to_string(), "?".to_string()));
    format!(
        "active={:?} target_active={} map={} opacity={} pos=({},{}) size={}x{} override_redirect={}",
        active,
        active == Some(wid),
        map_state(conn, wid),
        opacity,
        x,
        y,
        width,
        height,
        window_is_override_redirect(conn, wid)
    )
}

pub fn configure_overlay_window(title: &str) -> bool {
    let Some((conn, _screen_num, root, list_atom, name_atom, utf8_atom)) = connect_with_atoms()
    else {
        return false;
    };
    let Some(wid) = resolve_window(&conn, root, list_atom, name_atom, utf8_atom, title) else {
        return false;
    };

    set_window_manager_decorations(&conn, wid, false);
    lock_window_size(&conn, wid);
    add_window_state(&conn, root, wid);
    let _ = conn.flush();
    activate_window(&conn, root, wid);
    let _ = conn.flush();
    true
}

pub fn configure_pinned_window(title: &str) -> bool {
    let Some((conn, _screen_num, root, list_atom, name_atom, utf8_atom)) = connect_with_atoms()
    else {
        return false;
    };
    let Some(wid) = resolve_window(&conn, root, list_atom, name_atom, utf8_atom, title) else {
        return false;
    };

    set_window_manager_decorations(&conn, wid, false);
    keep_content_on_resize(&conn, wid);
    opt_out_of_sync_resize(&conn, wid);
    add_window_state(&conn, root, wid);
    let _ = conn.flush();
    true
}

pub fn pinned_window_kind() -> gpui::WindowKind {
    gpui::WindowKind::PopUp
}

fn keep_content_on_resize(conn: &impl Connection, wid: u32) {
    let attributes = ChangeWindowAttributesAux::new().bit_gravity(Gravity::NORTH_WEST);
    let _ = conn.change_window_attributes(wid, &attributes);
}

fn opt_out_of_sync_resize(conn: &impl Connection, wid: u32) {
    let Some(protocols_atom) = intern(conn, b"WM_PROTOCOLS") else {
        return;
    };
    let Some(sync_atom) = intern(conn, b"_NET_WM_SYNC_REQUEST") else {
        return;
    };
    let Some(property) = conn
        .get_property(false, wid, protocols_atom, AtomEnum::ATOM, 0, 32)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
    else {
        return;
    };
    let Some(atoms) = property.value32() else {
        return;
    };
    let kept: Vec<u32> = atoms.filter(|atom| *atom != sync_atom).collect();
    let _ = conn.change_property32(
        PropMode::REPLACE,
        wid,
        protocols_atom,
        AtomEnum::ATOM,
        &kept,
    );
}

fn lock_window_size(conn: &impl Connection, wid: u32) {
    let Ok(cookie) = conn.get_geometry(wid) else {
        return;
    };
    let Ok(geometry) = cookie.reply() else {
        return;
    };
    let size = (geometry.width as i32, geometry.height as i32);
    let _ = set_window_fixed_size(conn, wid, size);
}

fn set_window_fixed_size(conn: &impl Connection, wid: u32, size: (i32, i32)) -> bool {
    let hints = x11rb::properties::WmSizeHints {
        min_size: Some(size),
        max_size: Some(size),
        ..Default::default()
    };
    hints
        .set(conn, wid, AtomEnum::WM_NORMAL_HINTS)
        .ok()
        .and_then(|cookie| cookie.check().ok())
        .is_some()
}

pub fn set_window_fixed_size_by_title(title: &str, size: gpui::Size<gpui::Pixels>) -> bool {
    let Some((conn, _screen_num, root, list_atom, name_atom, utf8_atom)) = connect_with_atoms()
    else {
        return false;
    };
    let Some(wid) = resolve_window(&conn, root, list_atom, name_atom, utf8_atom, title) else {
        return false;
    };
    let size = (
        size.width.to_f64().round().max(1.0) as i32,
        size.height.to_f64().round().max(1.0) as i32,
    );
    set_window_fixed_size(&conn, wid, size) && conn.flush().is_ok()
}

fn add_window_state(conn: &impl Connection, root: u32, wid: u32) {
    let Some(state_atom) = intern(conn, b"_NET_WM_STATE") else {
        return;
    };
    const ADD: u32 = 1;
    const SOURCE_APPLICATION: u32 = 1;
    let states = [
        intern(conn, b"_NET_WM_STATE_ABOVE"),
        intern(conn, b"_NET_WM_STATE_SKIP_TASKBAR"),
        intern(conn, b"_NET_WM_STATE_SKIP_PAGER"),
    ];
    let mask = EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT;
    for atom in states.into_iter().flatten() {
        let event =
            ClientMessageEvent::new(32, wid, state_atom, [ADD, atom, 0, SOURCE_APPLICATION, 0]);
        let _ = conn.send_event(false, root, mask, event);
    }
}

fn clear_panel_window_state(conn: &impl Connection, root: u32, wid: u32) -> bool {
    let Some(state_atom) = intern(conn, b"_NET_WM_STATE") else {
        return false;
    };
    const REMOVE: u32 = 0;
    const SOURCE_APPLICATION: u32 = 1;
    let states = [
        intern(conn, b"_NET_WM_STATE_ABOVE"),
        intern(conn, b"_NET_WM_STATE_SKIP_TASKBAR"),
        intern(conn, b"_NET_WM_STATE_SKIP_PAGER"),
        intern(conn, b"_NET_WM_STATE_DEMANDS_ATTENTION"),
    ];
    let mask = EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT;
    let mut cleared = true;
    for atom in states.into_iter().flatten() {
        let event = ClientMessageEvent::new(
            32,
            wid,
            state_atom,
            [REMOVE, atom, 0, SOURCE_APPLICATION, 0],
        );
        cleared &= conn
            .send_event(false, root, mask, event)
            .ok()
            .and_then(|cookie| cookie.check().ok())
            .is_some();
    }
    cleared
}

fn activate_window(conn: &impl Connection, root: u32, wid: u32) -> (bool, u32) {
    let Some(active_atom) = intern(conn, b"_NET_ACTIVE_WINDOW") else {
        return (false, 0);
    };
    const SOURCE_PAGER: u32 = 2;
    let timestamp = server_activation_time(conn, root);
    let event = ClientMessageEvent::new(32, wid, active_atom, [SOURCE_PAGER, timestamp, 0, 0, 0]);
    let mask = EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT;
    (
        conn.send_event(false, root, mask, event)
            .ok()
            .and_then(|cookie| cookie.check().ok())
            .is_some(),
        timestamp,
    )
}

pub fn disable_window_shadow(_title: &str) -> bool {
    true
}

pub fn configure_popup_window(title: &str) -> bool {
    let Some((conn, _screen_num, root, list_atom, name_atom, utf8_atom)) = connect_with_atoms()
    else {
        return false;
    };
    let Some(wid) = resolve_window(&conn, root, list_atom, name_atom, utf8_atom, title) else {
        return false;
    };

    set_window_type_dock(&conn, wid);
    set_qol_ghost(&conn, wid);
    set_window_manager_decorations(&conn, wid, false);
    set_window_manager_state(&conn, wid);
    let _ = conn.flush();
    true
}

pub fn set_window_type_dock_by_title(title: &str) -> bool {
    let Some((conn, _screen_num, root, list_atom, name_atom, utf8_atom)) = connect_with_atoms()
    else {
        return false;
    };
    let Some(wid) = resolve_window(&conn, root, list_atom, name_atom, utf8_atom, title) else {
        return false;
    };
    set_window_type_dock(&conn, wid);
    conn.flush().is_ok()
}

pub fn make_override_redirect(title: &str) -> bool {
    let Some((conn, _screen_num, root, list_atom, name_atom, utf8_atom)) = connect_with_atoms()
    else {
        return false;
    };
    let Some(wid) = resolve_window(&conn, root, list_atom, name_atom, utf8_atom, title) else {
        return false;
    };
    let already = window_is_override_redirect(&conn, wid);
    if !already {
        let aux = ChangeWindowAttributesAux::new().override_redirect(1);
        let _ = conn.unmap_window(wid);
        let _ = conn.change_window_attributes(wid, &aux);
        let _ = conn.map_window(wid);
    }
    let stack = raise_window(&conn, root, wid);
    #[cfg(not(debug_assertions))]
    let _ = (&stack.frame, &stack.client, &stack.frame_ok);
    let _ = conn.flush();
    if !already {
        qol_runtime::probe!(
            "PICKER_OVERLAY",
            "title={title} wid={wid} override_redirect=1 frame={} stack_client={} stack_frame={}",
            stack.frame,
            stack.client,
            stack.frame_ok
        );
    }
    true
}

struct RaiseResult {
    frame: u32,
    client: bool,
    frame_ok: bool,
}

fn raise_window(conn: &impl Connection, root: u32, wid: u32) -> RaiseResult {
    let frame = top_level_frame(conn, root, wid).unwrap_or(wid);
    let client = configure_above(conn, wid);
    let frame_ok = if frame == wid {
        client
    } else {
        configure_above(conn, frame)
    };
    RaiseResult {
        frame,
        client,
        frame_ok,
    }
}

fn configure_above(conn: &impl Connection, wid: u32) -> bool {
    conn.configure_window(wid, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE))
        .ok()
        .and_then(|cookie| cookie.check().ok())
        .is_some()
}

fn window_is_override_redirect(conn: &impl Connection, wid: u32) -> bool {
    conn.get_window_attributes(wid)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|attrs| attrs.override_redirect)
        .unwrap_or(false)
}

static FOCUS_RETURN: Mutex<Option<u32>> = Mutex::new(None);

pub fn capture_focus_return() {
    let Some((conn, _screen_num, root, _list_atom, _name_atom, _utf8_atom)) = connect_with_atoms()
    else {
        return;
    };
    let target = active_window(&conn, root);
    if let Ok(mut slot) = FOCUS_RETURN.lock() {
        *slot = target;
    }
    qol_runtime::probe!("FOCUS_RETURN", "phase=captured target={target:?}");
}

fn take_input_focus(
    conn: &impl Connection,
    root: u32,
    wid: u32,
    active_before: Option<u32>,
) -> bool {
    let previous = focus_return_target(wid, active_before, active_window(conn, root));
    if let (Some(previous), Ok(mut slot)) = (previous, FOCUS_RETURN.lock()) {
        *slot = Some(previous);
    }
    if !window_is_override_redirect(conn, wid) {
        return false;
    }
    conn.set_input_focus(InputFocus::PARENT, wid, x11rb::CURRENT_TIME)
        .ok()
        .and_then(|cookie| cookie.check().ok())
        .is_some()
}

pub fn focus_window_by_title(title: &str) -> bool {
    let Some((conn, _screen_num, root, list_atom, name_atom, utf8_atom)) = connect_with_atoms()
    else {
        return false;
    };
    let Some(wid) = resolve_window(&conn, root, list_atom, name_atom, utf8_atom, title) else {
        return false;
    };
    take_input_focus(&conn, root, wid, None)
}

pub fn window_holds_input_focus(title: &str) -> Option<bool> {
    let (conn, _screen_num, root, list_atom, name_atom, utf8_atom) = connect_with_atoms()?;
    let wid = resolve_window(&conn, root, list_atom, name_atom, utf8_atom, title)?;
    let focus = conn.get_input_focus().ok()?.reply().ok()?.focus;
    Some(focus == wid)
}

pub fn release_focus_by_title(title: &str) {
    let Some((conn, _screen_num, root, list_atom, name_atom, utf8_atom)) = connect_with_atoms()
    else {
        return;
    };
    let Some(wid) = resolve_window(&conn, root, list_atom, name_atom, utf8_atom, title) else {
        return;
    };
    release_input_focus(&conn, root, wid);
}

fn focus_return_target(
    wid: u32,
    active_before: Option<u32>,
    active_current: Option<u32>,
) -> Option<u32> {
    active_before
        .filter(|&active| active != wid)
        .or_else(|| active_current.filter(|&active| active != wid))
}

fn release_input_focus(conn: &impl Connection, root: u32, wid: u32) {
    let holds = conn
        .get_input_focus()
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .map(|reply| reply.focus == wid)
        .unwrap_or(false);
    if !holds {
        return;
    }
    let target = FOCUS_RETURN
        .lock()
        .ok()
        .and_then(|mut slot| slot.take())
        .unwrap_or_else(|| u32::from(InputFocus::POINTER_ROOT));
    let activated = if target == u32::from(InputFocus::POINTER_ROOT) {
        false
    } else {
        activate_window(conn, root, target).0
    };
    let focused = conn
        .set_input_focus(InputFocus::PARENT, target, x11rb::CURRENT_TIME)
        .ok()
        .and_then(|cookie| cookie.check().ok())
        .is_some();
    let flushed = conn.flush().is_ok();
    #[cfg(not(debug_assertions))]
    let _ = (&activated, &focused, &flushed);
    qol_runtime::probe!(
        "FOCUS_RETURN",
        "from={wid} to={target} activated={activated} focused={focused} flushed={flushed}"
    );
}

struct CompositeLease {
    holders: Vec<String>,
    forced: Vec<(u32, Option<u32>)>,
}

impl CompositeLease {
    fn needs_force(&self, wid: u32) -> bool {
        !self.forced.iter().any(|(forced_wid, _)| *forced_wid == wid)
    }

    fn hold(&mut self, owner: &str, forced: Option<(u32, Option<u32>)>) {
        if let Some(entry) = forced {
            self.forced.push(entry);
        }
        if !self.holders.iter().any(|holder| holder == owner) {
            self.holders.push(owner.to_string());
        }
    }

    fn release(&mut self, owner: &str) -> Vec<(u32, Option<u32>)> {
        self.holders.retain(|holder| holder != owner);
        if self.holders.is_empty() {
            std::mem::take(&mut self.forced)
        } else {
            Vec::new()
        }
    }
}

static COMPOSITE_LEASE: Mutex<CompositeLease> = Mutex::new(CompositeLease {
    holders: Vec::new(),
    forced: Vec::new(),
});

const BYPASS_OFF: u32 = 2;

pub fn force_composite_below(owner: &str) {
    let Ok((conn, screen_num)) = x11rb::connect(None) else {
        return;
    };
    let root = conn.setup().roots[screen_num].root;
    let Some(active) = active_window(&conn, root) else {
        return;
    };
    if !window_is_fullscreen(&conn, active) {
        return;
    }
    let Ok(mut lease) = COMPOSITE_LEASE.lock() else {
        return;
    };
    let mut forced = None;
    if lease.needs_force(active) {
        let original = read_bypass_compositor(&conn, active);
        if original != Some(BYPASS_OFF) {
            set_bypass_compositor(&conn, active, BYPASS_OFF);
            let _ = conn.flush();
            forced = Some((active, original));
        }
    }
    lease.hold(owner, forced);
    qol_runtime::probe!(
        "FORCE_COMPOSITE",
        "owner={owner} wid={active} forced={forced:?}"
    );
}

pub fn restore_composite(owner: &str) {
    let forced = COMPOSITE_LEASE
        .lock()
        .map(|mut lease| lease.release(owner))
        .unwrap_or_default();
    if forced.is_empty() {
        return;
    }
    let Ok((conn, _screen_num)) = x11rb::connect(None) else {
        return;
    };
    for (wid, original) in forced {
        match original {
            Some(value) => set_bypass_compositor(&conn, wid, value),
            None => clear_bypass_compositor(&conn, wid),
        }
        qol_runtime::probe!(
            "RESTORE_COMPOSITE",
            "owner={owner} wid={wid} to={original:?}"
        );
    }
    let _ = conn.flush();
}

fn active_window(conn: &impl Connection, root: u32) -> Option<u32> {
    let atom = intern(conn, b"_NET_ACTIVE_WINDOW")?;
    let reply = conn
        .get_property(false, root, atom, AtomEnum::WINDOW, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    let wid = reply.value32()?.next()?;
    (wid != 0).then_some(wid)
}

fn window_is_fullscreen(conn: &impl Connection, wid: u32) -> bool {
    let (Some(state_atom), Some(fullscreen_atom)) = (
        intern(conn, b"_NET_WM_STATE"),
        intern(conn, b"_NET_WM_STATE_FULLSCREEN"),
    ) else {
        return false;
    };
    let Ok(reply) = conn.get_property(false, wid, state_atom, AtomEnum::ATOM, 0, 64) else {
        return false;
    };
    let Ok(prop) = reply.reply() else {
        return false;
    };
    prop.value32()
        .map(|mut atoms| atoms.any(|atom| atom == fullscreen_atom))
        .unwrap_or(false)
}

fn read_bypass_compositor(conn: &impl Connection, wid: u32) -> Option<u32> {
    let atom = intern(conn, b"_NET_WM_BYPASS_COMPOSITOR")?;
    let reply = conn
        .get_property(false, wid, atom, AtomEnum::CARDINAL, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    reply.value32().and_then(|mut values| values.next())
}

fn set_bypass_compositor(conn: &impl Connection, wid: u32, value: u32) {
    if let Some(atom) = intern(conn, b"_NET_WM_BYPASS_COMPOSITOR") {
        let _ = conn.change_property32(PropMode::REPLACE, wid, atom, AtomEnum::CARDINAL, &[value]);
    }
}

fn clear_bypass_compositor(conn: &impl Connection, wid: u32) {
    if let Some(atom) = intern(conn, b"_NET_WM_BYPASS_COMPOSITOR") {
        let _ = conn.delete_property(wid, atom);
    }
}

const SERVER_TIME_PROBE_BUDGET: Duration = Duration::from_millis(50);
const SERVER_TIME_PROBE_COOLDOWN: Duration = Duration::from_secs(5);

struct ServerTime {
    anchor: Option<(u32, Instant)>,
    cooldown_until: Option<Instant>,
}

static SERVER_TIME: Mutex<ServerTime> = Mutex::new(ServerTime {
    anchor: None,
    cooldown_until: None,
});

fn server_activation_time(conn: &impl Connection, root: u32) -> u32 {
    let now = Instant::now();
    let Ok(mut state) = SERVER_TIME.lock() else {
        return 0;
    };
    if let Some((server_ms, at)) = state.anchor {
        return server_ms.wrapping_add(now.saturating_duration_since(at).as_millis() as u32);
    }
    if state.cooldown_until.is_some_and(|until| now < until) {
        return 0;
    }
    match probe_server_time(conn, root) {
        Some(server_ms) => {
            state.anchor = Some((server_ms, now));
            server_ms
        }
        None => {
            state.cooldown_until = Some(now + SERVER_TIME_PROBE_COOLDOWN);
            0
        }
    }
}

fn probe_server_time(conn: &impl Connection, root: u32) -> Option<u32> {
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
    let probe = intern(conn, b"_QOL_POPUP_TIME")?;
    let empty: &[u8] = &[];
    conn.change_property8(PropMode::APPEND, window, probe, AtomEnum::STRING, empty)
        .ok()?;
    conn.flush().ok()?;

    let deadline = Instant::now() + SERVER_TIME_PROBE_BUDGET;
    let time = loop {
        if let Some(event) = conn.poll_for_event().ok()? {
            if let Event::PropertyNotify(notify) = event {
                if notify.window == window {
                    break Some(notify.time);
                }
            }
            continue;
        }
        if Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(1));
    };
    let _ = conn.destroy_window(window);
    let _ = conn.flush();
    time
}

pub fn set_ghost_debug(opacity: Option<f32>, _color_hex: Option<&str>) {
    GHOST_ALPHA.store(normalize_opacity(opacity).to_bits(), Ordering::Relaxed);
}

pub fn window_backing_scale(_title: &str) -> Option<f32> {
    None
}

pub fn dump_ghost_windows(context: &str) {
    #[cfg(debug_assertions)]
    {
        let context = context.to_string();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(60));
            let Ok((conn, screen_num)) = x11rb::connect(None) else {
                crate::probe::probe("GHOSTDUMP", &format!("ctx={context} x11=unavailable"));
                return;
            };
            let root = conn.setup().roots[screen_num].root;
            let (Some(name_atom), Some(utf8_atom), Some(list_atom)) = (
                intern(&conn, b"_NET_WM_NAME"),
                intern(&conn, b"UTF8_STRING"),
                intern(&conn, b"_NET_CLIENT_LIST"),
            ) else {
                return;
            };
            let mut ids = root_window_ids(&conn, root, list_atom);
            append_tree_window_ids(&conn, root, &mut ids);
            crate::probe::probe("GHOSTDUMP", &format!("ctx={context} begin"));
            let mut count = 0u32;
            for wid in ids {
                if !is_qol_ghost(&conn, wid) {
                    continue;
                }
                let Some(title) = read_window_name(&conn, wid, name_atom, utf8_atom) else {
                    continue;
                };
                crate::probe::probe("GHOSTWIN", &inspect_ghost_window(&conn, root, wid, &title));
                count += 1;
            }
            crate::probe::probe("GHOSTDUMP", &format!("ctx={context} end n={count}"));
        });
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = context;
    }
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

fn set_window_type_dock(conn: &impl Connection, wid: u32) {
    let Some(type_atom) = intern(conn, b"_NET_WM_WINDOW_TYPE") else {
        return;
    };
    let Some(dock_atom) = intern(conn, b"_NET_WM_WINDOW_TYPE_DOCK") else {
        return;
    };
    let _ = conn.change_property32(
        PropMode::REPLACE,
        wid,
        type_atom,
        AtomEnum::ATOM,
        &[dock_atom],
    );
}

fn set_qol_ghost(conn: &impl Connection, wid: u32) {
    let Some(atom) = intern(conn, b"_QOL_GHOST") else {
        return;
    };
    let _ = conn.change_property32(PropMode::REPLACE, wid, atom, AtomEnum::CARDINAL, &[1]);
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

fn ghost_opacity() -> f32 {
    f32::from_bits(GHOST_ALPHA.load(Ordering::Relaxed))
}

fn normalize_opacity(opacity: Option<f32>) -> f32 {
    let value = opacity.unwrap_or(0.0);
    if !value.is_finite() {
        return 0.0;
    }
    value.clamp(0.0, 1.0)
}

fn opacity_to_cardinal(opacity: f32) -> u32 {
    (normalize_opacity(Some(opacity)) * u32::MAX as f32).round() as u32
}

fn set_window_opacity(conn: &impl Connection, wid: u32, opacity: f32) -> bool {
    let Some(atom) = intern(conn, b"_NET_WM_WINDOW_OPACITY") else {
        return false;
    };
    let value = opacity_to_cardinal(opacity);
    conn.change_property32(PropMode::REPLACE, wid, atom, AtomEnum::CARDINAL, &[value])
        .ok()
        .and_then(|cookie| cookie.check().ok())
        .is_some()
}

fn clear_window_opacity(conn: &impl Connection, wid: u32) -> bool {
    let Some(atom) = intern(conn, b"_NET_WM_WINDOW_OPACITY") else {
        return false;
    };
    conn.delete_property(wid, atom)
        .ok()
        .and_then(|cookie| cookie.check().ok())
        .is_some()
}

fn clear_window_type(conn: &impl Connection, wid: u32) -> bool {
    let Some(type_atom) = intern(conn, b"_NET_WM_WINDOW_TYPE") else {
        return false;
    };
    let Some(normal_atom) = intern(conn, b"_NET_WM_WINDOW_TYPE_NORMAL") else {
        return false;
    };
    conn.change_property32(
        PropMode::REPLACE,
        wid,
        type_atom,
        AtomEnum::ATOM,
        &[normal_atom],
    )
    .ok()
    .and_then(|cookie| cookie.check().ok())
    .is_some()
}

#[cfg(debug_assertions)]
fn read_window_name(
    conn: &impl Connection,
    wid: u32,
    name_atom: u32,
    utf8_atom: u32,
) -> Option<String> {
    let reply = conn
        .get_property(false, wid, name_atom, utf8_atom, 0, 256)
        .ok()?
        .reply()
        .ok()?;
    if reply.value.is_empty() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&reply.value)
            .trim_end_matches('\0')
            .to_string(),
    )
}

#[cfg(debug_assertions)]
fn inspect_ghost_window(conn: &impl Connection, root: u32, wid: u32, title: &str) -> String {
    let owner = window_pid(conn, wid)
        .map(|pid| pid.to_string())
        .unwrap_or_else(|| "?".to_string());
    let (x, y, w, h) = absolute_geometry(conn, root, wid).unwrap_or((i32::MIN, i32::MIN, 0, 0));
    let opacity = window_opacity(conn, wid);
    let opacity_str = opacity
        .map(|value| format!("{value:.2}"))
        .unwrap_or_else(|| "unset".to_string());
    let map = map_state(conn, wid);
    let role = ghost_role(map, opacity);
    format!(
        "title={title} owner_pid={owner} wid={wid} pos=({x},{y}) size={w}x{h} opacity={opacity_str} map={map} role={role}"
    )
}

#[cfg(debug_assertions)]
fn ghost_role(map: &str, opacity: Option<f32>) -> &'static str {
    if map != "viewable" {
        return "hidden";
    }
    match opacity {
        Some(value) if value <= 0.01 => "invisible",
        Some(value) if value < 0.99 => "ghost",
        _ => "live",
    }
}

#[cfg(debug_assertions)]
fn window_pid(conn: &impl Connection, wid: u32) -> Option<u32> {
    let atom = intern(conn, b"_NET_WM_PID")?;
    let reply = conn
        .get_property(false, wid, atom, AtomEnum::CARDINAL, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    reply.value32().and_then(|mut values| values.next())
}

#[cfg(debug_assertions)]
fn is_qol_ghost(conn: &impl Connection, wid: u32) -> bool {
    let Some(atom) = intern(conn, b"_QOL_GHOST") else {
        return false;
    };
    conn.get_property(false, wid, atom, AtomEnum::CARDINAL, 0, 1)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .and_then(|reply| reply.value32()?.next())
        == Some(1)
}

fn window_opacity(conn: &impl Connection, wid: u32) -> Option<f32> {
    let atom = intern(conn, b"_NET_WM_WINDOW_OPACITY")?;
    let reply = conn
        .get_property(false, wid, atom, AtomEnum::CARDINAL, 0, 1)
        .ok()?
        .reply()
        .ok()?;
    let raw = reply.value32()?.next()?;
    Some(raw as f32 / u32::MAX as f32)
}

fn window_is_visible(conn: &impl Connection, wid: u32) -> bool {
    let viewable = conn
        .get_window_attributes(wid)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .is_some_and(|attributes| attributes.map_state == MapState::VIEWABLE);
    viewable && window_opacity(conn, wid).unwrap_or(1.0) > 0.01
}

fn map_state(conn: &impl Connection, wid: u32) -> &'static str {
    let Ok(cookie) = conn.get_window_attributes(wid) else {
        return "err";
    };
    let Ok(attrs) = cookie.reply() else {
        return "err";
    };
    if attrs.map_state == MapState::VIEWABLE {
        "viewable"
    } else if attrs.map_state == MapState::UNMAPPED {
        "unmapped"
    } else if attrs.map_state == MapState::UNVIEWABLE {
        "unviewable"
    } else {
        "unknown"
    }
}

fn absolute_geometry(conn: &impl Connection, root: u32, wid: u32) -> Option<(i32, i32, u16, u16)> {
    let geometry = conn.get_geometry(wid).ok()?.reply().ok()?;
    let translated = conn
        .translate_coordinates(wid, root, 0, 0)
        .ok()?
        .reply()
        .ok()?;
    Some((
        translated.dst_x as i32,
        translated.dst_y as i32,
        geometry.width,
        geometry.height,
    ))
}

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

fn set_input_passthrough(conn: &impl Connection, wid: u32, passthrough: bool) -> bool {
    if !passthrough {
        return shape::mask(
            conn,
            shape::SO::SET,
            shape::SK::INPUT,
            wid,
            0,
            0,
            x11rb::NONE,
        )
        .is_ok();
    }
    shape::rectangles(
        conn,
        shape::SO::SET,
        shape::SK::INPUT,
        ClipOrdering::UNSORTED,
        wid,
        0,
        0,
        &[],
    )
    .is_ok()
}

fn set_keepalive_input_hint(conn: &impl Connection, wid: u32) -> bool {
    let existing = WmHints::get(conn, wid)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
        .flatten()
        .unwrap_or_default();
    let hints = keepalive_wm_hints(existing);
    hints
        .set(conn, wid)
        .ok()
        .and_then(|cookie| cookie.check().ok())
        .is_some()
}

fn keepalive_wm_hints(mut hints: WmHints) -> WmHints {
    hints.input = Some(false);
    hints
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

#[cfg(test)]
mod tests {
    use super::{
        composite_owner, focus_return_target, keepalive_wm_hints, normalize_opacity,
        opacity_to_cardinal, pinned_window_kind, CompositeLease,
    };
    use gpui::WindowKind;
    use x11rb::properties::{WmHints, WmHintsState};

    #[test]
    fn pinned_windows_are_popups_before_their_first_map() {
        assert_eq!(pinned_window_kind(), WindowKind::PopUp);
    }

    #[test]
    fn composite_owner_strips_ghost_geometry_suffix() {
        let cases = [
            ("foo@0,0,1920x1080", "foo"),
            ("foo-pin-123-0", "foo-pin-123-0"),
            ("foo@1,2,3x4@extra", "foo"),
        ];
        for (title, expected) in cases {
            assert_eq!(composite_owner(title), expected, "title: {title}");
        }
    }

    #[test]
    fn keepalive_hints_refuse_focus_without_clobbering_other_hints() {
        let original = WmHints {
            input: Some(true),
            initial_state: Some(WmHintsState::Iconic),
            icon_pixmap: Some(1),
            icon_window: Some(2),
            icon_position: Some((3, 4)),
            icon_mask: Some(5),
            window_group: Some(6),
            urgent: true,
        };

        let configured = keepalive_wm_hints(original);

        assert_eq!(configured.input, Some(false));
        assert!(matches!(
            configured.initial_state,
            Some(WmHintsState::Iconic)
        ));
        assert_eq!(configured.icon_pixmap, Some(1));
        assert_eq!(configured.icon_window, Some(2));
        assert_eq!(configured.icon_position, Some((3, 4)));
        assert_eq!(configured.icon_mask, Some(5));
        assert_eq!(configured.window_group, Some(6));
        assert!(configured.urgent);
    }

    #[test]
    fn focus_return_prefers_the_window_active_before_the_panel() {
        let cases = [
            (7, Some(3), Some(7), Some(3)),
            (7, None, Some(3), Some(3)),
            (7, Some(7), Some(3), Some(3)),
            (7, None, Some(7), None),
            (7, None, None, None),
        ];
        for (wid, before, current, expected) in cases {
            assert_eq!(
                focus_return_target(wid, before, current),
                expected,
                "wid={wid} before={before:?} current={current:?}"
            );
        }
    }

    #[test]
    fn composite_lease_restores_only_after_last_holder_releases() {
        let mut lease = CompositeLease {
            holders: Vec::new(),
            forced: Vec::new(),
        };
        lease.hold("pin-1", Some((7, None)));
        assert!(!lease.needs_force(7), "wid 7 already forced");
        lease.hold("preview", None);
        lease.hold("preview", None);
        assert_eq!(
            lease.release("preview"),
            Vec::new(),
            "pin-1 still holds the lease"
        );
        assert_eq!(
            lease.release("pin-1"),
            vec![(7, None)],
            "last holder triggers restore"
        );
        assert!(lease.needs_force(7), "forced list drained after restore");
    }

    #[test]
    fn composite_lease_release_of_unknown_owner_keeps_forced_windows() {
        let mut lease = CompositeLease {
            holders: Vec::new(),
            forced: Vec::new(),
        };
        lease.hold("pin-1", Some((7, Some(1))));
        lease.hold("pin-2", Some((9, None)));
        assert_eq!(lease.release("stranger"), Vec::new(), "holders remain");
        assert_eq!(
            lease.release("pin-1"),
            Vec::new(),
            "pin-2 still holds the lease"
        );
        assert_eq!(
            lease.release("pin-2"),
            vec![(7, Some(1)), (9, None)],
            "all forced windows restored together"
        );
    }

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
