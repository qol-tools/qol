use crate::discovery::WindowDiscovery;
use crate::picker::create::PICKER_WINDOW_TITLE;
use std::sync::atomic::{AtomicU64, Ordering};

static FOCUS_REASSERT_GEN: AtomicU64 = AtomicU64::new(0);

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
    qol_gpui::ghost::show_ghost_window_topmost(target_title, all_titles);
    let generation = FOCUS_REASSERT_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    qol_gpui::popup_window::reassert_focus_until_held(
        target_title,
        &FOCUS_REASSERT_GEN,
        generation,
    );
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
    current: &crate::picker::PickerWindowState,
    preview_cache: crate::picker::run::SharedPreviewCache,
    tracker: &qol_gpui::monitor::MonitorTracker,
    cx: &mut gpui::App,
) {
    qol_gpui::popup_window::set_ghost_debug(
        config.display.ghost_opacity,
        config.display.ghost_debug_color.as_deref(),
    );
    let windows = crate::discovery::Platform
        .visible_windows(
            config.display.show_minimized,
            &crate::config::SwitchablePanels::resolve(&config.switchable_panels),
        )
        .unwrap_or_default();
    let monitors = tracker.all_monitors();
    if !monitors.is_empty() {
        for monitor in monitors {
            let placement = qol_gpui::window::PopupPlacement::from_monitor(Some(monitor));
            crate::picker::create::pre_create_ghost(
                config,
                current,
                &placement,
                preview_cache.clone(),
                &windows,
                cx,
            );
        }
        return;
    }
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
    _current: &crate::picker::PickerWindowState,
    _target: qol_gpui::window::MonitorKey,
    _cx: &mut gpui::App,
) {
}

pub fn discard_old_window(
    current: &crate::picker::PickerWindowState,
    target: qol_gpui::window::MonitorKey,
    handle: gpui::WindowHandle<crate::app::AltTabApp>,
    cx: &mut gpui::App,
) {
    let _ = handle.update(cx, |_, window, _| window.remove_window());
    current.borrow_mut().remove(target);
}
