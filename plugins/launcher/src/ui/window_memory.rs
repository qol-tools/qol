use gpui::{px, size, App, Bounds, Pixels, Size, Window};
use qol_gpui::placement::MonitorPlacement;
use qol_gpui::surface::WindowMemory;

use crate::monitor::ActiveMonitor;

use super::layout::{min_height, MIN_WIDTH};

const MEMORY_KEY: &str = "launcher";
const PLACEMENT: MonitorPlacement = MonitorPlacement::center();

pub(super) fn min_size() -> Size<Pixels> {
    size(px(MIN_WIDTH), px(min_height()))
}

pub(super) fn user_size(monitor: Option<&ActiveMonitor>) -> Option<Size<Pixels>> {
    WindowMemory::size_and_place(MEMORY_KEY).user_size(
        PLACEMENT,
        min_size(),
        monitor.map(ActiveMonitor::logical_work_area),
    )
}

pub(super) fn remember(user: Size<Pixels>, window: &Window, cx: &mut App) {
    WindowMemory::size_and_place(MEMORY_KEY).record_user_size(
        PLACEMENT,
        Bounds::new(window.bounds().origin, user),
        window.scale_factor(),
        cx,
    );
}
