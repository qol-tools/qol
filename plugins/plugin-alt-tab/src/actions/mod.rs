mod platform;

pub fn activate_window(window_id: u32) {
    platform::activate_window(window_id)
}

pub fn close_window(window_id: u32) {
    platform::close_window(window_id)
}

pub fn quit_app(window_id: u32) {
    platform::quit_app(window_id)
}

pub fn minimize_window_by_id(window_id: u32) {
    platform::minimize_window_by_id(window_id)
}
