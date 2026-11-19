#[cfg(any(target_os = "macos", target_os = "windows"))]
mod bin;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(unix))]
mod portable;
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod trash_reason;
#[cfg(unix)]
mod unix;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) use fallback::{copy_link, free_bytes, name_problem, removable, system_bin, volume_of};
#[cfg(target_os = "linux")]
pub(crate) use linux::system_bin;
#[cfg(target_os = "macos")]
pub(crate) use macos::{name_problem, system_bin};
#[cfg(not(unix))]
pub(crate) use portable::{born_at, changed_at, keep_mode, new_dir, new_file, open_dir};
#[cfg(target_os = "linux")]
pub(crate) use unix::name_problem;
#[cfg(unix)]
pub(crate) use unix::{
    born_at, changed_at, copy_link, free_bytes, keep_mode, new_dir, new_file, open_dir, removable,
    volume_of,
};
#[cfg(target_os = "windows")]
pub(crate) use windows::{copy_link, free_bytes, name_problem, removable, system_bin, volume_of};
