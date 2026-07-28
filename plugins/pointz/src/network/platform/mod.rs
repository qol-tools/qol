#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix_fd;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(super) use fallback::adopt_inherited_udp;
#[cfg(target_os = "linux")]
pub(super) use linux::adopt_inherited_udp;
#[cfg(target_os = "macos")]
pub(super) use macos::adopt_inherited_udp;
#[cfg(target_os = "windows")]
pub(super) use windows::adopt_inherited_udp;
