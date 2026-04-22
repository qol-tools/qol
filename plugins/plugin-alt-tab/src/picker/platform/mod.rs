#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
pub(crate) mod macos;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;

#[cfg(target_os = "windows")]
mod imp {
    pub fn picker_window_kind() -> gpui::WindowKind {
        gpui::WindowKind::PopUp
    }
    pub fn dismiss_picker(window: &mut gpui::Window) {
        window.minimize_window();
    }
    pub fn set_accessory_policy() {}
    pub fn reposition_picker_window(_x: f64, _y: f64) -> bool {
        false
    }
    pub fn is_modifier_held() -> bool {
        false
    }
    pub fn is_shift_held() -> bool {
        false
    }
    pub fn disable_window_shadow() {}
    pub fn show_picker_onscreen() {}
    pub fn prepare_picker_for_show() {}
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("plugin-alt-tab picker: unsupported target OS");

pub fn picker_window_kind() -> gpui::WindowKind {
    imp::picker_window_kind()
}
pub fn dismiss_picker(window: &mut gpui::Window) {
    imp::dismiss_picker(window)
}
pub fn set_accessory_policy() {
    imp::set_accessory_policy()
}
pub fn reposition_picker_window(x: f64, y: f64) -> bool {
    imp::reposition_picker_window(x, y)
}
pub fn is_modifier_held() -> bool {
    imp::is_modifier_held()
}
#[allow(dead_code)]
pub fn is_shift_held() -> bool {
    imp::is_shift_held()
}
pub fn disable_window_shadow() {
    imp::disable_window_shadow()
}
/// Fade the picker to alpha=1 after `activate_window()` has returned. macOS-only; no-op on other platforms.
pub fn show_picker_onscreen() {
    imp::show_picker_onscreen()
}
/// Force the picker to alpha=0 before repositioning/activation. macOS-only; no-op on other platforms.
pub fn prepare_picker_for_show() {
    imp::prepare_picker_for_show()
}
/// Hide the picker without destroying its NSWindow (bootstrap path). macOS-only.
#[cfg(target_os = "macos")]
pub fn hide_picker_offscreen() {
    imp::hide_picker_offscreen()
}
