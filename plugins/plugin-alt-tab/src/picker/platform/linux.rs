use crate::discovery::WindowDiscovery;
use crate::picker::create::PICKER_WINDOW_TITLE;

pub fn picker_window_title(target: qol_gpui::window::MonitorKey) -> String {
    format!(
        "{}@{},{},{}x{}",
        PICKER_WINDOW_TITLE, target.x, target.y, target.width, target.height
    )
}

pub fn picker_window_kind() -> gpui::WindowKind {
    qol_gpui::platform::ghost_window_kind()
}

pub fn configure_picker_window(title: &str) {
    qol_gpui::popup_window::configure_popup_window(title);
}

pub fn show_picker_window(target_title: &str, all_titles: &[String]) {
    qol_gpui::ghost::show_ghost_window(target_title, all_titles);
}

pub fn reuse_hidden_picker_across_shows() -> bool {
    true
}

pub fn reuse_picker_across_targets() -> bool {
    false
}

pub fn disable_window_shadow(title: &str) {
    qol_gpui::popup_window::disable_window_shadow(title);
}

pub fn probe_picker_app_active(_at: &'static str) {}

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
    let state = qol_gpui::PlatformStateClient::from_env().get_state();
    let monitors = state
        .map(|state| state.monitors)
        .filter(|monitors| !monitors.is_empty());
    if let Some(monitors) = monitors {
        for monitor in monitors {
            let placement = qol_gpui::window::PopupPlacement::from_monitor(Some(
                qol_gpui::monitor::ActiveMonitor::from_bounds(monitor),
            ));
            crate::picker::create::pre_create_ghost(config, current, &placement, &windows, cx);
        }
        return;
    }
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
    handle: gpui::WindowHandle<crate::app::AltTabApp>,
    cx: &mut gpui::App,
) {
    let _ = handle.update(cx, |_, window, _| window.remove_window());
    current.borrow_mut().remove(target);
}
