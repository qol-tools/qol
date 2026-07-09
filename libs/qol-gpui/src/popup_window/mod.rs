mod platform;

use std::cell::RefCell;

use crate::runtime_config::load_gpui_runtime_config;

#[cfg(target_os = "linux")]
pub use platform::{
    force_composite_below, make_override_redirect, window_geometry_session,
    window_position_by_title, WindowGeometrySession,
};

#[cfg(target_os = "linux")]
pub fn restore_composite() {
    platform::restore_composite()
}

#[cfg(not(target_os = "linux"))]
pub fn restore_composite() {}
pub use platform::{
    configure_overlay_window, configure_pinned_window, configure_popup_window,
    disable_window_shadow, dump_ghost_windows, hide_for_capture, hide_invisible,
    hide_window_by_title, hide_windows_by_title_prefix, reposition_window_by_title,
    show_window_by_title, sync_window_layout, visible_windows_by_title_prefix,
    window_backing_scale,
};

const ENV_GHOST_OPACITY: &str = "QOL_TRAY_GHOST_OPACITY";
const ENV_GHOST_COLOR: &str = "QOL_TRAY_GHOST_COLOR";

pub fn set_ghost_debug(opacity: Option<f32>, color_hex: Option<&str>) {
    let runtime = load_gpui_runtime_config();
    let opacity = ghost_opacity_env().or(runtime.ghost_opacity).or(opacity);
    let runtime_color = runtime.ghost_debug_color;
    let env_color = ghost_color_env();
    let color_hex = env_color
        .as_deref()
        .or(runtime_color.as_deref())
        .or(color_hex);
    platform::set_ghost_debug(opacity, color_hex);
}

fn ghost_opacity_env() -> Option<f32> {
    std::env::var(ENV_GHOST_OPACITY)
        .ok()
        .and_then(|raw| raw.trim().parse::<f32>().ok())
}

fn ghost_color_env() -> Option<String> {
    std::env::var(ENV_GHOST_COLOR).ok()
}

thread_local! {
    static CHANGE_REASON: RefCell<String> = const { RefCell::new(String::new()) };
}

pub(crate) fn change_reason() -> String {
    CHANGE_REASON.with(|cell| {
        let reason = cell.borrow();
        if reason.is_empty() {
            "?".to_string()
        } else {
            reason.clone()
        }
    })
}

pub struct ReasonScope(String);

pub fn reason_scope(reason: impl Into<String>) -> ReasonScope {
    let reason = reason.into();
    ReasonScope(CHANGE_REASON.with(|cell| std::mem::replace(&mut *cell.borrow_mut(), reason)))
}

impl Drop for ReasonScope {
    fn drop(&mut self) {
        CHANGE_REASON.with(|cell| *cell.borrow_mut() = std::mem::take(&mut self.0));
    }
}
