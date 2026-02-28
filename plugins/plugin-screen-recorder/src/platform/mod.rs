mod linux;

#[cfg(not(target_os = "linux"))]
compile_error!("plugin-screen-recorder: only Linux is supported");

pub use linux::*;

pub const SETTINGS_URL: &str = "http://127.0.0.1:42700/plugins/plugin-screen-recorder/";
pub const CAPTURE_LOG: &str = "/tmp/record-region.log";
