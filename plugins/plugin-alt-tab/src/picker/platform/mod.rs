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
    use qol_gpui::window::MonitorKey;
    pub fn picker_window_kind() -> gpui::WindowKind {
        gpui::WindowKind::PopUp
    }
    pub fn picker_window_decorations(_transparent: bool) -> gpui::WindowDecorations {
        gpui::WindowDecorations::Client
    }
    pub fn set_accessory_policy() {}
    pub fn picker_window_title(_target: MonitorKey) -> String {
        "qol-alt-tab-picker".to_string()
    }
    pub fn configure_picker_window(_title: &str) {}
    pub fn sync_picker_window_layout(
        _title: &str,
        window: &mut gpui::Window,
        _origin: gpui::Point<gpui::Pixels>,
        size: gpui::Size<gpui::Pixels>,
    ) -> bool {
        qol_gpui::window::resize_or_sync_scale(window, size, None);
        true
    }
    pub fn adjust_picker_bounds(bounds: gpui::Bounds<gpui::Pixels>) -> gpui::Bounds<gpui::Pixels> {
        bounds
    }
    pub fn reuse_hidden_picker_across_shows() -> bool {
        true
    }
    pub fn reuse_picker_across_targets() -> bool {
        true
    }
    pub fn is_modifier_held() -> bool {
        false
    }
    pub fn is_shift_held() -> bool {
        false
    }
    pub fn disable_window_shadow(_title: &str) {}
    pub fn show_picker(_title: &str) {}
    pub fn hide_picker(_title: &str) {}
    pub fn pre_create(
        _config: &AltTabConfig,
        _current: &PickerWindowState,
        _tracker: &qol_gpui::monitor::MonitorTracker,
        _cx: &mut gpui::App,
    ) {
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
pub fn picker_window_decorations(transparent: bool) -> gpui::WindowDecorations {
    imp::picker_window_decorations(transparent)
}
pub fn set_accessory_policy() {
    imp::set_accessory_policy()
}
pub fn picker_window_title(target: qol_gpui::window::MonitorKey) -> String {
    imp::picker_window_title(target)
}
pub fn configure_picker_window(title: &str) {
    imp::configure_picker_window(title)
}
pub fn sync_picker_window_layout(
    title: &str,
    window: &mut gpui::Window,
    origin: gpui::Point<gpui::Pixels>,
    size: gpui::Size<gpui::Pixels>,
) -> bool {
    imp::sync_picker_window_layout(title, window, origin, size)
}
pub fn adjust_picker_bounds(bounds: gpui::Bounds<gpui::Pixels>) -> gpui::Bounds<gpui::Pixels> {
    imp::adjust_picker_bounds(bounds)
}
pub fn reuse_hidden_picker_across_shows() -> bool {
    imp::reuse_hidden_picker_across_shows()
}
pub fn reuse_picker_across_targets() -> bool {
    imp::reuse_picker_across_targets()
}
pub fn is_modifier_held() -> bool {
    imp::is_modifier_held()
}
#[allow(dead_code)]
pub fn is_shift_held() -> bool {
    imp::is_shift_held()
}
pub fn disable_window_shadow(title: &str) {
    imp::disable_window_shadow(title)
}
pub fn show_picker(title: &str) {
    imp::show_picker(title)
}
pub fn hide_picker(title: &str) {
    imp::hide_picker(title)
}
pub fn pre_create(
    config: &crate::config::AltTabConfig,
    current: &crate::PickerWindowState,
    tracker: &qol_gpui::monitor::MonitorTracker,
    cx: &mut gpui::App,
) {
    imp::pre_create(config, current, tracker, cx)
}

/// Destroy sibling picker windows on non-target monitors. macOS keeps a single
/// keep-alive picker that gets repositioned, so this is a no-op there.
pub fn destroy_non_target_windows(
    current: &crate::PickerWindowState,
    target: qol_gpui::window::MonitorKey,
    cx: &mut gpui::App,
) {
    imp::destroy_non_target_windows(current, target, cx)
}

/// Discard a picker window handle whose reuse attempt failed. macOS only drops
/// the `ActiveWindows` slot where the platform keep-alive window must live; others
/// remove the underlying window too.
pub fn discard_old_window(
    current: &crate::PickerWindowState,
    target: qol_gpui::window::MonitorKey,
    handle: gpui::WindowHandle<crate::app::AltTabApp>,
    cx: &mut gpui::App,
) {
    imp::discard_old_window(current, target, handle, cx)
}
