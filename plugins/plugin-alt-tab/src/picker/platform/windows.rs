use crate::app::AltTabApp;
use crate::config::AltTabConfig;
use crate::PickerWindowState;
use qol_gpui::window::MonitorKey;

pub fn picker_window_title(_target: MonitorKey) -> String {
    "qol-alt-tab-picker".to_string()
}

pub fn configure_picker_window(_title: &str) {}

pub fn reuse_hidden_picker_across_shows() -> bool {
    true
}

pub fn reuse_picker_across_targets() -> bool {
    true
}

pub fn disable_window_shadow(_title: &str) {}

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
