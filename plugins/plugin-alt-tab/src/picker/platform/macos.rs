use crate::discovery::WindowDiscovery;
use crate::picker::create::PICKER_WINDOW_TITLE;

pub fn picker_window_title(_target: qol_gpui::window::MonitorKey) -> String {
    PICKER_WINDOW_TITLE.to_string()
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
    true
}

pub fn disable_window_shadow(title: &str) {
    qol_gpui::popup_window::disable_window_shadow(title);
}

pub fn probe_picker_app_active(at: &'static str) {
    #[cfg(debug_assertions)]
    {
        use objc2_app_kit::NSApplication;
        use objc2_foundation::MainThreadMarker;

        let Some(mtm) = MainThreadMarker::new() else {
            qol_runtime::probe!(
                "PICKER_APP_ACTIVE",
                "active=unknown at={at} reason=not-main-thread"
            );
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        qol_runtime::probe!("PICKER_APP_ACTIVE", "active={} at={at}", app.isActive());
    }
    #[cfg(not(debug_assertions))]
    let _ = at;
}

pub fn pre_create(
    config: &crate::config::AltTabConfig,
    current: &crate::PickerWindowState,
    preview_cache: crate::picker::run::SharedPreviewCache,
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
    crate::picker::create::pre_create_ghost(
        config,
        current,
        &placement,
        preview_cache,
        &windows,
        cx,
    );
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
