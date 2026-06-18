pub fn activate_window(_window_id: u32) {}
pub fn close_window(_window_id: u32) -> super::CloseWindowResult {
    super::CloseWindowResult::Unsupported
}
pub fn quit_app(_window_id: u32) {}
pub fn minimize_window_by_id(_window_id: u32) {}
