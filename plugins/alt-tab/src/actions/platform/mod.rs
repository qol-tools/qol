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
    QuitApp,
    Unsupported,
}

pub(crate) enum CloseOutcome {
    Closed { quit_app: bool },
    Unsupported,
}

pub fn activate_window(window_id: u32) {
    imp::activate_window(window_id)
}
pub fn cancel_pending_activation() {
    imp::cancel_pending_activation()
}
pub fn close_window(window_id: u32) -> CloseWindowResult {
    match imp::close_window(window_id) {
        CloseOutcome::Closed { quit_app: true } => CloseWindowResult::QuitApp,
        CloseOutcome::Closed { quit_app: false } => CloseWindowResult::ClosedWindow,
        CloseOutcome::Unsupported => CloseWindowResult::Unsupported,
    }
}
pub fn quit_app(window_id: u32) {
    imp::quit_app(window_id)
}
pub fn minimize_window_by_id(window_id: u32) {
    imp::minimize_window_by_id(window_id)
}
