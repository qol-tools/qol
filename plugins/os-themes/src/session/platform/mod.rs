pub trait SessionPlatform {
    fn exit_restores_host(&self) -> bool;
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(any(target_os = "linux", target_os = "windows"))]
mod store;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
pub use fallback::Platform;
#[cfg(target_os = "linux")]
pub use linux::Platform;
#[cfg(target_os = "macos")]
pub use macos::Platform;
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(crate) use store::session_subdir;
#[cfg(target_os = "windows")]
pub use windows::Platform;
