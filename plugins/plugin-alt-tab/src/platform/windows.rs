use super::WindowInfo;

pub fn cached_preview_path(_window_id: u32) -> Option<String> {
    None
}

pub fn get_open_windows() -> Vec<WindowInfo> {
    Vec::new()
}

pub fn capture_preview(_window_id: u32, _max_w: usize, _max_h: usize) -> Option<String> {
    None
}

pub fn capture_previews_batch(
    _targets: &[(usize, u32)],
    _max_w: usize,
    _max_h: usize,
) -> Vec<(usize, Option<String>)> {
    Vec::new()
}

pub fn activate_window(_window_id: u32) {}
