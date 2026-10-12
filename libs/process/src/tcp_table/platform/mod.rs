#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod fallback;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub use fallback::{tcp_connections, tcp_listeners};
#[cfg(target_os = "linux")]
pub use linux::{tcp_connections, tcp_listeners};
#[cfg(target_os = "macos")]
pub use macos::{tcp_connections, tcp_listeners};
#[cfg(windows)]
pub use windows::{tcp_connections, tcp_listeners};
