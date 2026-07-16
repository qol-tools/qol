mod platform;

use std::cell::RefCell;

use crate::runtime_config::load_gpui_runtime_config;

#[cfg(target_os = "linux")]
pub use platform::{
    focus_window_by_title, make_override_redirect, release_focus_by_title, window_geometry_session,
    window_position_by_title, WindowGeometrySession,
};

#[cfg(target_os = "linux")]
pub fn present_topmost(title: &str) {
    platform::force_composite_below(composite_owner(title));
    platform::make_override_redirect(title);
}

#[cfg(not(target_os = "linux"))]
pub fn present_topmost(_title: &str) {}

#[cfg(target_os = "linux")]
pub fn restore_composite(title: &str) {
    platform::restore_composite(composite_owner(title))
}

#[cfg(not(target_os = "linux"))]
pub fn restore_composite(_title: &str) {}

#[cfg(target_os = "linux")]
fn composite_owner(title: &str) -> &str {
    title.split('@').next().unwrap_or(title)
}
pub use platform::{
    configure_overlay_window, configure_pinned_window, configure_popup_window,
    disable_window_shadow, dump_ghost_windows, hide_for_capture, hide_invisible,
    hide_window_by_title, hide_windows_by_title_prefix, reposition_window_by_title,
    show_window_by_title, show_window_passive_by_title, sync_window_layout,
    visible_windows_by_title_prefix, window_backing_scale, window_holds_input_focus,
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

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::composite_owner;

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
}
