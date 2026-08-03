use qol_windowing::{WindowId, WindowOps, WindowRect};

pub struct Platform;

impl WindowOps for Platform {
    fn enumerate_windows(&self) -> Result<Vec<WindowId>, String> {
        Err(unsupported())
    }

    fn window_geometry(&self, _window_id: &WindowId) -> Result<Option<WindowRect>, String> {
        Err(unsupported())
    }

    fn move_resize(&self, _window_id: &WindowId, _rect: WindowRect) -> Result<(), String> {
        Err(unsupported())
    }

    fn focus_window(&self, _window_id: &WindowId) -> Result<bool, String> {
        Err(unsupported())
    }

    fn minimize_window(&self, _window_id: &WindowId) -> Result<bool, String> {
        Err(unsupported())
    }

    fn restore_window(&self, _window_id: &WindowId) -> Result<bool, String> {
        Err(unsupported())
    }
}

fn unsupported() -> String {
    "alt-tab: window actions are not implemented on Windows".to_string()
}

pub fn cancel_pending_activation() {}

pub fn close_window(_window_id: u32) -> super::CloseOutcome {
    super::CloseOutcome::Unsupported
}

pub fn quit_app(_window_id: u32) {}
