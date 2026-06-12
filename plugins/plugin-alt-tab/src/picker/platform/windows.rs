use crate::app::AltTabApp;
use crate::config::AltTabConfig;
use crate::PickerWindowState;
use qol_gpui::window::MonitorKey;

pub fn picker_window_title(_target: MonitorKey) -> String {
    "qol-alt-tab-picker".to_string()
}

pub fn picker_window_kind() -> gpui::WindowKind {
    qol_gpui::platform::ghost_window_kind()
}

pub fn configure_picker_window(_title: &str) {}

pub fn show_picker_window(target_title: &str, all_titles: &[String]) {
    qol_gpui::ghost::show_ghost_window(target_title, all_titles);
}

pub fn reuse_hidden_picker_across_shows() -> bool {
    true
}

pub fn reuse_picker_across_targets() -> bool {
    true
}

pub fn disable_window_shadow(_title: &str) {}

pub fn probe_picker_app_active(_at: &'static str) {}

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
