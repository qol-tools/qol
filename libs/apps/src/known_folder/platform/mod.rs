#[cfg(not(target_os = "windows"))]
mod fallback;
#[cfg(target_os = "windows")]
pub(crate) mod windows;

#[cfg(not(target_os = "windows"))]
pub use fallback::user_content_folders;
#[cfg(target_os = "windows")]
pub use windows::user_content_folders;
