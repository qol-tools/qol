#[cfg(not(target_os = "windows"))]
mod fallback;
#[cfg(target_os = "windows")]
pub(crate) mod windows;

#[cfg(not(target_os = "windows"))]
pub use fallback::{pictures_dir, programs_dir, user_content_folders, videos_dir};
#[cfg(target_os = "windows")]
pub use windows::{pictures_dir, programs_dir, user_content_folders, videos_dir};
