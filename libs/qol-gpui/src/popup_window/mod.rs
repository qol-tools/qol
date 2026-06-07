mod platform;

use std::cell::Cell;

use crate::runtime_config::load_gpui_runtime_config;

pub use platform::{
    configure_popup_window, disable_window_shadow, dump_ghost_windows, hide_window_by_title,
    reposition_window_by_title, show_window_by_title, window_backing_scale,
};

#[cfg(target_os = "linux")]
pub use platform::{hide_window_invisible, set_window_bounds_by_title};

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
    static CHANGE_REASON: Cell<&'static str> = const { Cell::new("?") };
}

pub struct ReasonScope(&'static str);

pub fn reason_scope(reason: &'static str) -> ReasonScope {
    ReasonScope(CHANGE_REASON.with(|cell| cell.replace(reason)))
}

impl Drop for ReasonScope {
    fn drop(&mut self) {
        CHANGE_REASON.with(|cell| cell.set(self.0));
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn change_reason() -> &'static str {
    CHANGE_REASON.with(|cell| cell.get())
}
