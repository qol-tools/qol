#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(target_os = "windows"))]
mod rename_check;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix_call;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub use fallback::{file_identity, rename_noreplace};
#[cfg(target_os = "linux")]
pub use linux::{file_identity, rename_noreplace};
#[cfg(target_os = "macos")]
pub use macos::{file_identity, rename_noreplace};
#[cfg(target_os = "windows")]
pub use windows::{file_identity, rename_noreplace};

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub(crate) use fallback::{
    ensure_private_file, prepare_file_removal, set_mode, set_private, set_private_dir, sync_parent,
};
#[cfg(target_os = "linux")]
pub(crate) use linux::{
    ensure_private_file, prepare_file_removal, set_mode, set_private, set_private_dir, sync_parent,
};
#[cfg(target_os = "macos")]
pub(crate) use macos::{
    ensure_private_file, prepare_file_removal, set_mode, set_private, set_private_dir, sync_parent,
};
#[cfg(target_os = "windows")]
pub(crate) use windows::{
    ensure_private_file, prepare_file_removal, set_mode, set_private, set_private_dir, sync_parent,
};
