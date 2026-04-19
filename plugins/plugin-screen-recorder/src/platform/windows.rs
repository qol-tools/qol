use anyhow::{anyhow, Result};
use std::path::Path;

use crate::{Config, Monitor, Rect};

pub fn select_region() -> Result<Option<Rect>> {
    Err(anyhow!(
        "plugin-screen-recorder: region selection is not implemented on Windows"
    ))
}

pub fn get_monitors() -> Result<Vec<Monitor>> {
    Err(anyhow!(
        "plugin-screen-recorder: monitor enumeration is not implemented on Windows"
    ))
}

pub fn full_screen_bounds() -> Result<Monitor> {
    Err(anyhow!(
        "plugin-screen-recorder: full screen bounds are not implemented on Windows"
    ))
}

pub fn start_capture(_rect: &Rect, _config: &Config, _output_file: &Path) -> Result<u32> {
    Err(anyhow!(
        "plugin-screen-recorder: capture start is not implemented on Windows"
    ))
}

pub fn stop_capture(_pid: u32) -> Result<()> {
    Err(anyhow!(
        "plugin-screen-recorder: capture stop is not implemented on Windows"
    ))
}

pub fn process_alive(_pid: u32) -> bool {
    false
}

pub fn show_notification(_title: &str, _message: &str, _timeout_ms: u32) {
    // Notifications are fire-and-forget UX; silently no-op on Windows.
}

pub fn open_settings() -> Result<()> {
    Err(anyhow!(
        "plugin-screen-recorder: settings launcher is not implemented on Windows"
    ))
}
