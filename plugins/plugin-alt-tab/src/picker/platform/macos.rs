use crate::discovery::WindowDiscovery;
use crate::picker::create::PICKER_WINDOW_TITLE;

pub fn hide_picker(title: &str) {
    qol_gpui::popup_window::hide_window_by_title(title);
}

pub fn picker_window_title(_target: qol_gpui::window::MonitorKey) -> String {
    PICKER_WINDOW_TITLE.to_string()
}

pub fn configure_picker_window(title: &str) {
    qol_gpui::popup_window::configure_popup_window(title);
}

pub fn reposition_picker_window(title: &str, gpui_x: f64, gpui_y: f64) -> bool {
    qol_gpui::popup_window::reposition_window_by_title(title, gpui_x, gpui_y)
}

pub fn sync_picker_window_layout(
    title: &str,
    window: &mut gpui::Window,
    origin: gpui::Point<gpui::Pixels>,
    size: gpui::Size<gpui::Pixels>,
) -> bool {
    let backing = qol_gpui::popup_window::window_backing_scale(title);
    qol_gpui::window::resize_or_sync_scale(window, size, backing);
    reposition_picker_window(title, origin.x.to_f64(), origin.y.to_f64())
}

pub fn reuse_hidden_picker_across_shows() -> bool {
    true
}

pub fn reuse_picker_across_targets() -> bool {
    true
}

pub fn disable_window_shadow(title: &str) {
    qol_gpui::popup_window::disable_window_shadow(title);
}

pub fn pre_create(
    config: &crate::config::AltTabConfig,
    current: &crate::PickerWindowState,
    tracker: &qol_gpui::monitor::MonitorTracker,
    cx: &mut gpui::App,
) {
    qol_gpui::popup_window::set_ghost_debug(
        config.display.ghost_opacity,
        config.display.ghost_debug_color.as_deref(),
    );
    let windows = crate::discovery::Platform
        .visible_windows(config.display.show_minimized)
        .unwrap_or_default();
    let placement = qol_gpui::window::PopupPlacement::from_tracker(tracker);
    crate::picker::create::pre_create_ghost(config, current, &placement, &windows, cx);
}

pub fn destroy_non_target_windows(
    _current: &crate::PickerWindowState,
    _target: qol_gpui::window::MonitorKey,
    _cx: &mut gpui::App,
) {
}

pub fn discard_old_window(
    current: &crate::PickerWindowState,
    target: qol_gpui::window::MonitorKey,
    _handle: gpui::WindowHandle<crate::app::AltTabApp>,
    _cx: &mut gpui::App,
) {
    current.borrow_mut().remove(target);
}
