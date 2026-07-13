use anyhow::{Context, Result};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::{read_devices, InputMonitor};
#[cfg(target_os = "macos")]
pub use macos::{read_devices, InputMonitor};
#[cfg(target_os = "windows")]
pub use windows::{read_devices, InputMonitor};

pub struct NativeInputSnapshot {
    pub available: bool,
    pub source: Option<&'static str>,
    pub items: Vec<NativeControllerInput>,
}

pub struct NativeControllerInput {
    pub name: String,
    pub vendor: u16,
    pub product: u16,
    pub connection: NativeConnection,
    pub buttons: Vec<NativeButtonInput>,
}

pub struct NativeConnection {
    pub transport: &'static str,
    pub signal_dbm: Option<i16>,
}

pub struct NativeButtonInput {
    pub index: usize,
    pub pressed: bool,
}

const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");

pub fn open_settings() -> Result<()> {
    let settings_url = qol_conventions::settings_url(PLUGIN_ID);
    qol_apps::desktop_integration::open_with_default_app(&settings_url)
        .context("failed to open settings URL")
}
