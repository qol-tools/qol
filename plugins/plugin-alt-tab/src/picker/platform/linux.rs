use crate::picker::create::PICKER_WINDOW_TITLE;
use std::sync::{Mutex, OnceLock};
use x11rb::connection::Connection;
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

pub fn reposition_picker_window(gpui_x: f64, gpui_y: f64) -> bool {
    qol_gpui::popup_window::reposition_window_by_title(PICKER_WINDOW_TITLE, gpui_x, gpui_y)
}

pub fn disable_window_shadow() {
    qol_gpui::popup_window::disable_window_shadow(PICKER_WINDOW_TITLE);
}

pub fn show_picker() {
    qol_gpui::popup_window::show_window_by_title(PICKER_WINDOW_TITLE);
}

pub fn hide_picker() {
    qol_gpui::popup_window::hide_window_by_title(PICKER_WINDOW_TITLE);
}

pub fn pre_create(
    config: &crate::config::AltTabConfig,
    current: &crate::PickerWindowState,
    tracker: &qol_gpui::monitor::MonitorTracker,
    cx: &mut gpui::App,
) {
    let placement = qol_gpui::window::PopupPlacement::from_tracker(tracker);
    crate::picker::create::pre_create_ghost(config, current, &placement, cx)
}

pub fn destroy_non_target_windows(
    current: &crate::PickerWindowState,
    target: qol_gpui::window::MonitorKey,
    cx: &mut gpui::App,
) {
    current.borrow_mut().destroy_non_target(target, cx);
}

pub fn discard_old_window(
    current: &crate::PickerWindowState,
    target: qol_gpui::window::MonitorKey,
    handle: gpui::WindowHandle<crate::app::AltTabApp>,
    cx: &mut gpui::App,
) {
    #[cfg(debug_assertions)]
    eprintln!("[alt-tab/open] closing old window — will recreate on correct monitor");
    let _ = handle.update(cx, |_, window, _| window.remove_window());
    current.borrow_mut().remove(target);
}
