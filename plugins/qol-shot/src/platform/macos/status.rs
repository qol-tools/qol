use gpui::{App, Bounds, Pixels};

pub fn show_capture_status(
    _monitor_bounds: Bounds<Pixels>,
    _title: String,
    _subtitle: String,
    _cx: &mut App,
) -> bool {
    false
}

pub fn hide_capture_status(_cx: &mut App) {}
