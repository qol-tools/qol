#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) use fallback::{show_already_running, show_first_run};
#[cfg(target_os = "linux")]
pub(crate) use linux::{show_already_running, show_first_run};
#[cfg(target_os = "macos")]
pub(crate) use macos::{show_already_running, show_first_run};
