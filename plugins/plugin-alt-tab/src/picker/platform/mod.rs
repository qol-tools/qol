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
    use crate::app::AltTabApp;
    use crate::config::AltTabConfig;
    use crate::PickerWindowState;
    use qol_plugin_api::window::MonitorKey;
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
    pub fn hide_picker_offscreen() {}
    pub fn pre_create_if_supported(
        _config: &AltTabConfig,
        _current: &PickerWindowState,
        _cx: &mut gpui::App,
    ) {
    }
    pub fn offscreen_origin() -> (f64, f64) {
        (0.0, 0.0)
    }
    pub fn destroy_non_target_windows(
        _current: &PickerWindowState,
        _target: MonitorKey,
        _cx: &mut gpui::App,
    ) {
    }
    pub fn discard_old_window(
        current: &PickerWindowState,
        target: MonitorKey,
        _handle: gpui::WindowHandle<AltTabApp>,
        _cx: &mut gpui::App,
    ) {
        current.borrow_mut().remove(target);
    }
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
/// Hide the picker without destroying its NSWindow (bootstrap path). No-op where unsupported.
pub fn hide_picker_offscreen() {
    imp::hide_picker_offscreen()
}
/// Pre-create an offscreen picker window at boot so first show is instant.
/// No-op on platforms that create/destroy per show.
pub fn pre_create_if_supported(
    config: &crate::config::AltTabConfig,
    current: &crate::PickerWindowState,
    cx: &mut gpui::App,
) {
    imp::pre_create_if_supported(config, current, cx)
}

/// Offscreen origin for the keep-alive picker. Far off-monitor on macOS; (0, 0)
/// elsewhere (unused — pre_create_if_supported is a no-op there).
#[allow(dead_code)] // only consumed on macOS through create::pre_create_offscreen
pub fn offscreen_origin() -> (f64, f64) {
    imp::offscreen_origin()
}

/// Destroy sibling picker windows on non-target monitors. macOS keeps a single
/// keep-alive picker that gets repositioned, so this is a no-op there.
pub fn destroy_non_target_windows(
    current: &crate::PickerWindowState,
    target: qol_plugin_api::window::MonitorKey,
    cx: &mut gpui::App,
) {
    imp::destroy_non_target_windows(current, target, cx)
}

/// Discard a picker window handle whose reuse attempt failed. macOS only drops
/// the `ActiveWindows` slot (keep-alive NSWindow must live); others remove the
/// underlying window too.
pub fn discard_old_window(
    current: &crate::PickerWindowState,
    target: qol_plugin_api::window::MonitorKey,
    handle: gpui::WindowHandle<crate::app::AltTabApp>,
    cx: &mut gpui::App,
) {
    imp::discard_old_window(current, target, handle, cx)
}
