use gpui::App;

use crate::config::AltTabConfig;
use crate::picker::run::SharedPreviewCache;
use crate::picker::PickerWindowState;

pub(super) fn pre_create(
    config: &AltTabConfig,
    current: &PickerWindowState,
    preview_cache: SharedPreviewCache,
    tracker: &qol_gpui::monitor::MonitorTracker,
    cx: &mut App,
) {
    qol_gpui::popup_window::set_ghost_debug(
        config.display.ghost_opacity,
        config.display.ghost_debug_color.as_deref(),
    );
    let windows = crate::discovery::WindowDiscovery::visible_windows(
        &crate::discovery::Platform,
        config.display.show_minimized,
        &config.switchable_panels,
    )
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
