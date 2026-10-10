#[cfg(not(target_os = "windows"))]
mod fallback;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(target_os = "windows"))]
pub use fallback::shell_execute;
#[cfg(target_os = "windows")]
pub use windows::shell_execute;
