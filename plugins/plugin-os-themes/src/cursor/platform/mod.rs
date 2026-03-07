#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(not(target_os = "linux"))]
compile_error!("plugin-os-themes: cursor effects are not yet implemented for this platform");
