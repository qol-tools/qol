#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
pub(crate) mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(target_os = "windows")]
use windows as imp;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("plugin-alt-tab picker: unsupported target OS");

pub fn picker_window_kind() -> gpui::WindowKind {
    qol_gpui::platform::ghost_window_kind()
}
pub fn picker_window_decorations(transparent: bool) -> gpui::WindowDecorations {
    qol_gpui::platform::ghost_window_decorations(transparent)
}
pub fn set_accessory_policy() {
    qol_gpui::platform::set_accessory_policy()
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
    qol_gpui::ghost::sync_window_layout(title, window, origin, size)
}
pub fn adjust_picker_bounds(bounds: gpui::Bounds<gpui::Pixels>) -> gpui::Bounds<gpui::Pixels> {
    qol_gpui::platform::adjust_ghost_bounds(bounds)
}
pub fn reuse_hidden_picker_across_shows() -> bool {
    imp::reuse_hidden_picker_across_shows()
}
pub fn reuse_picker_across_targets() -> bool {
    imp::reuse_picker_across_targets()
}
pub fn is_modifier_held() -> bool {
    qol_gpui::platform::is_modifier_held()
}
#[allow(dead_code)]
pub fn is_shift_held() -> bool {
    qol_gpui::platform::is_shift_held()
}
pub fn disable_window_shadow(title: &str) {
    imp::disable_window_shadow(title)
}
pub fn pre_create(
    config: &crate::config::AltTabConfig,
    current: &crate::PickerWindowState,
    tracker: &qol_gpui::monitor::MonitorTracker,
    cx: &mut gpui::App,
) {
    imp::pre_create(config, current, tracker, cx)
}
pub fn destroy_non_target_windows(
    current: &crate::PickerWindowState,
    target: qol_gpui::window::MonitorKey,
    cx: &mut gpui::App,
) {
    imp::destroy_non_target_windows(current, target, cx)
}
pub fn discard_old_window(
    current: &crate::PickerWindowState,
    target: qol_gpui::window::MonitorKey,
    handle: gpui::WindowHandle<crate::app::AltTabApp>,
    cx: &mut gpui::App,
) {
    imp::discard_old_window(current, target, handle, cx)
}
