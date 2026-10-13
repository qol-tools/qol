#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod runtime_events;

#[cfg(target_os = "macos")]
pub(super) use macos::data_refresh_listener_loop;
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(super) use runtime_events::data_refresh_listener_loop;
