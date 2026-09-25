use qol_windowing::{WindowId, WindowOps};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;
#[cfg(target_os = "windows")]
use windows as imp;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloseWindowResult {
    ClosedWindow,
    CloseRequested,
    QuitApp,
    Unsupported,
}

pub(crate) enum CloseOutcome {
    Closed {
        quit_app: bool,
        awaits_destroy_event: bool,
    },
    Unsupported,
}

pub fn activate_window(window_id: u32) {
    let _ = imp::Platform.focus_window(&WindowId::from_u32(window_id));
}
pub fn cancel_pending_activation() {
    imp::cancel_pending_activation()
}
pub fn close_window(window_id: u32) -> CloseWindowResult {
    match imp::close_window(window_id) {
        CloseOutcome::Closed { quit_app: true, .. } => CloseWindowResult::QuitApp,
        CloseOutcome::Closed {
            awaits_destroy_event: true,
            ..
        } => CloseWindowResult::CloseRequested,
        CloseOutcome::Closed { .. } => CloseWindowResult::ClosedWindow,
        CloseOutcome::Unsupported => CloseWindowResult::Unsupported,
    }
}
pub fn quit_app(window_id: u32) {
    imp::quit_app(window_id)
}
pub fn destroyed_windows() -> Option<futures::channel::mpsc::UnboundedReceiver<u32>> {
    imp::destroyed_windows()
}
pub fn minimize_window_by_id(window_id: u32) {
    let _ = imp::Platform.minimize_window(&WindowId::from_u32(window_id));
}
