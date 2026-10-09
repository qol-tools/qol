use qol_windowing::platform::windows::{top_level_windows, Window};
use qol_windowing::{WindowId, WindowOps, WindowRect};

pub struct Platform;

fn window(window_id: &WindowId) -> Result<Window, String> {
    Window::from_id(window_id)
        .ok_or_else(|| format!("alt-tab: invalid window id {}", window_id.as_str()))
}

impl WindowOps for Platform {
    fn enumerate_windows(&self) -> Result<Vec<WindowId>, String> {
        Ok(top_level_windows()
            .into_iter()
            .filter(|window| window.is_switchable())
            .map(Window::id)
            .collect())
    }

    fn window_geometry(&self, window_id: &WindowId) -> Result<Option<WindowRect>, String> {
        let window = window(window_id)?;
        if !window.exists() {
            return Ok(None);
        }
        Ok(window.frame())
    }

    fn move_resize(&self, window_id: &WindowId, rect: WindowRect) -> Result<(), String> {
        window(window_id)?.set_frame(rect)
    }

    fn focus_window(&self, window_id: &WindowId) -> Result<bool, String> {
        Ok(window(window_id)?.activate())
    }

    fn minimize_window(&self, window_id: &WindowId) -> Result<bool, String> {
        Ok(window(window_id)?.minimize())
    }

    fn restore_window(&self, window_id: &WindowId) -> Result<bool, String> {
        Ok(window(window_id)?.activate())
    }
}

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
