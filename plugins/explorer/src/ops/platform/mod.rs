#[cfg(any(target_os = "linux", target_os = "windows"))]
mod bin;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(unix))]
mod portable;
#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
mod trash_reason;
#[cfg(unix)]
mod unix;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) use fallback::{
    copy_link, free_bytes, in_trash, name_problem, removable, restore_trashed, trash_locations,
    trash_one, volume_of,
};
#[cfg(target_os = "linux")]
pub(crate) use linux::{in_trash, restore_trashed, trash_locations, trash_one};
#[cfg(target_os = "macos")]
pub(crate) use macos::{in_trash, name_problem, restore_trashed, trash_locations, trash_one};
#[cfg(not(unix))]
pub(crate) use portable::{keep_mode, new_dir, new_file};
#[cfg(target_os = "linux")]
pub(crate) use unix::name_problem;
#[cfg(unix)]
pub(crate) use unix::{copy_link, free_bytes, keep_mode, new_dir, new_file, removable, volume_of};
#[cfg(target_os = "windows")]
pub(crate) use windows::{
    copy_link, free_bytes, in_trash, name_problem, removable, restore_trashed, trash_locations,
    trash_one, volume_of,
};
