use gpui::Window;

/// Hide the launcher by parking the persistent ghost at alpha=0 (and ignoring
/// mouse events) rather than destroying it, so the next show is instant.
pub fn hide(_window: &mut Window) {
    qol_gpui::popup_window::hide_window_by_title(super::LAUNCHER_WINDOW_TITLE);
}
