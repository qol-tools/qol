use crate::picker::create::PICKER_WINDOW_TITLE;
use std::sync::{Mutex, OnceLock};
use x11rb::protocol::xproto::ConnectionExt;
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

pub fn picker_window_decorations(_transparent: bool) -> gpui::WindowDecorations {
    gpui::WindowDecorations::Client
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

pub fn picker_window_title(target: qol_gpui::window::MonitorKey) -> String {
    format!(
        "{}@{},{},{}x{}",
        PICKER_WINDOW_TITLE, target.x, target.y, target.width, target.height
    )
}

pub fn configure_picker_window(title: &str) {
    qol_gpui::popup_window::configure_popup_window(title);
}

pub fn sync_picker_window_layout(
    title: &str,
    _window: &mut gpui::Window,
    origin: gpui::Point<gpui::Pixels>,
    size: gpui::Size<gpui::Pixels>,
) -> bool {
    qol_gpui::popup_window::set_window_bounds_by_title(
        title,
        origin.x.to_f64(),
        origin.y.to_f64(),
        size.width.to_f64(),
        size.height.to_f64(),
    )
}

pub fn adjust_picker_bounds(bounds: gpui::Bounds<gpui::Pixels>) -> gpui::Bounds<gpui::Pixels> {
    let x = bounds.origin.x.to_f64() + 1.0;
    let y = bounds.origin.y.to_f64() + 1.0;
    let width = (bounds.size.width.to_f64() - 2.0).max(1.0);
    let height = (bounds.size.height.to_f64() - 2.0).max(1.0);
    gpui::Bounds::new(
        gpui::point(gpui::px(x as f32), gpui::px(y as f32)),
        gpui::size(gpui::px(width as f32), gpui::px(height as f32)),
    )
}

pub fn reuse_hidden_picker_across_shows() -> bool {
    true
}

pub fn reuse_picker_across_targets() -> bool {
    false
}

pub fn disable_window_shadow(title: &str) {
    qol_gpui::popup_window::disable_window_shadow(title);
}

pub fn show_picker(title: &str) {
    qol_gpui::popup_window::show_window_by_title(title);
}

pub fn hide_picker(title: &str) {
    qol_gpui::popup_window::hide_window_by_title(title);
}

pub fn pre_create(
    config: &crate::config::AltTabConfig,
    current: &crate::PickerWindowState,
    tracker: &qol_gpui::monitor::MonitorTracker,
    cx: &mut gpui::App,
) {
    qol_gpui::popup_window::set_ghost_debug(
        config.display.ghost_opacity,
        config.display.ghost_debug_color.as_deref(),
    );
    let state = qol_gpui::PlatformStateClient::from_env().get_state();
    let monitors = state
        .map(|state| state.monitors)
        .filter(|monitors| !monitors.is_empty());
    if let Some(monitors) = monitors {
        for monitor in monitors {
            let placement = qol_gpui::window::PopupPlacement::from_monitor(Some(
                qol_gpui::monitor::ActiveMonitor::from_bounds(monitor),
            ));
            crate::picker::create::pre_create_ghost(config, current, &placement, cx);
        }
        return;
    }
    let placement = qol_gpui::window::PopupPlacement::from_tracker(tracker);
    crate::picker::create::pre_create_ghost(config, current, &placement, cx);
}

pub fn destroy_non_target_windows(
    _current: &crate::PickerWindowState,
    _target: qol_gpui::window::MonitorKey,
    _cx: &mut gpui::App,
) {
}

pub fn discard_old_window(
    current: &crate::PickerWindowState,
    target: qol_gpui::window::MonitorKey,
    handle: gpui::WindowHandle<crate::app::AltTabApp>,
    cx: &mut gpui::App,
) {
    #[cfg(debug_assertions)]
    eprintln!(
        "[alt-tab/open] keep-alive reuse failed; dropping stale slot {:?}",
        target
    );
    let _ = handle.update(cx, |_, window, _| window.remove_window());
    current.borrow_mut().remove(target);
}
