#[cfg(not(any(unix, target_os = "windows")))]
mod fallback;
#[cfg(unix)]
mod unix;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(unix, target_os = "windows")))]
pub(in crate::plugins) use fallback::DaemonListener;
#[cfg(not(any(unix, target_os = "windows")))]
pub(in crate::plugins::daemon_lifecycle) use fallback::{
    apply_to_command, bind_for_plugin, refresh_for_respawn,
};
#[cfg(unix)]
pub(in crate::plugins) use unix::DaemonListener;
#[cfg(unix)]
pub(in crate::plugins::daemon_lifecycle) use unix::{
    apply_to_command, bind_for_plugin, refresh_for_respawn,
};
#[cfg(target_os = "windows")]
pub(in crate::plugins) use windows::DaemonListener;
#[cfg(target_os = "windows")]
pub(in crate::plugins::daemon_lifecycle) use windows::{
    apply_to_command, bind_for_plugin, refresh_for_respawn,
};
