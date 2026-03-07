#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "linux")]
pub use linux::{create_effect, install_signal_handlers, open_settings};

#[cfg(not(target_os = "linux"))]
compile_error!("plugin-os-themes: cursor effects are not yet implemented for this platform");
