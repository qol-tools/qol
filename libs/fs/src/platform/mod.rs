#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) use fallback::{
    prepare_file_removal, set_mode, set_private, set_private_dir, sync_parent,
};
#[cfg(target_os = "linux")]
pub(crate) use linux::{prepare_file_removal, set_mode, set_private, set_private_dir, sync_parent};
#[cfg(target_os = "macos")]
pub(crate) use macos::{prepare_file_removal, set_mode, set_private, set_private_dir, sync_parent};
#[cfg(target_os = "windows")]
pub(crate) use windows::{
    prepare_file_removal, set_mode, set_private, set_private_dir, sync_parent,
};
