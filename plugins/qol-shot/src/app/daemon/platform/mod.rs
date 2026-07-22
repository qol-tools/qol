#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(unix)]
mod unix_common;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::{cleanup, start_listener, wait_and_send_action};
#[cfg(target_os = "macos")]
pub(crate) use macos::{cleanup, start_listener, wait_and_send_action};
#[cfg(target_os = "windows")]
pub(crate) use windows::{cleanup, start_listener, wait_and_send_action};
