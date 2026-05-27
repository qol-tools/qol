#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub use fallback::{has_process_focus, should_poll_process_focus};
#[cfg(target_os = "linux")]
pub use linux::{has_process_focus, should_poll_process_focus};
#[cfg(target_os = "macos")]
pub use macos::{has_process_focus, should_poll_process_focus};
