use gpui::*;
use std::rc::Rc;

pub fn show_capture_status(
    monitor_bounds: Bounds<Pixels>,
    title: String,
    subtitle: String,
    cx: &mut App,
) -> bool {
    let bounds = crate::ui::region_selector::guide_panel_bounds(monitor_bounds);
    let reveal: crate::ui::region_selector::SelectorReveal = Rc::new(|title| {
        super::window::configure_status_window(&title);
    });
    super::SELECTOR_CACHE.with(|cache| {
        crate::ui::region_selector::platform::show_cached_guide(
            cache,
            bounds,
            title.into(),
            subtitle.into(),
            reveal,
            cx,
        )
        .is_some()
    })
}

pub fn hide_capture_status(cx: &mut App) {
    super::SELECTOR_CACHE.with(|cache| {
        crate::ui::region_selector::platform::hide_cached_guide(cache, cx);
    });
}
