#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::open_dir;
#[cfg(target_os = "macos")]
pub(crate) use macos::open_dir;
#[cfg(target_os = "windows")]
pub(crate) use windows::open_dir;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("sync::platform::open_dir is not implemented for this target");
