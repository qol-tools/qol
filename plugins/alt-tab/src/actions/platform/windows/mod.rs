use qol_windowing::platform::windows::{top_level_windows, Window};
use qol_windowing::WindowId;

pub use qol_windowing::platform::windows::Win32Windows as Platform;

pub fn cancel_pending_activation() {}

pub fn close_window(window_id: u32) -> super::CloseOutcome {
    let closed = Window::from_id(&WindowId::from_u32(window_id)).is_some_and(Window::request_close);
    if !closed {
        return super::CloseOutcome::Unsupported;
    }
    super::CloseOutcome::Closed {
        quit_app: false,
        awaits_destroy_event: false,
    }
}

pub fn quit_app(window_id: u32) {
    let Some(pid) = Window::from_id(&WindowId::from_u32(window_id)).and_then(Window::pid) else {
        return;
    };
    for window in top_level_windows()
        .into_iter()
        .filter(|window| window.pid() == Some(pid) && window.is_switchable())
    {
        window.request_close();
    }
}

pub fn destroyed_windows() -> Option<futures::channel::mpsc::UnboundedReceiver<u32>> {
    None
}
