#[cfg(unix)]
mod unix;
#[cfg(not(any(unix, windows)))]
mod unsupported;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub(super) use unix::{
    inherited_listener, is_executable, open_settings, spawn_host_death_watchdog,
};
#[cfg(not(any(unix, windows)))]
pub(super) use unsupported::{
    inherited_listener, is_executable, open_settings, spawn_host_death_watchdog,
};
#[cfg(windows)]
pub(super) use windows::{
    inherited_listener, is_executable, open_settings, spawn_host_death_watchdog,
};
