use anyhow::{Context, Result};

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
pub(crate) use linux::platform_support;
#[cfg(target_os = "linux")]
pub use linux::{read_devices, InputMonitor};
#[cfg(target_os = "macos")]
pub(crate) use macos::platform_support;
#[cfg(target_os = "macos")]
pub use macos::{read_devices, InputMonitor};
#[cfg(target_os = "windows")]
pub(crate) use windows::platform_support;
#[cfg(target_os = "windows")]
pub use windows::{read_devices, InputMonitor};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PlatformSupport {
    pub(crate) label: &'static str,
    pub(crate) supported: bool,
}

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
    pub signal: Option<NativeSignal>,
    pub adapter: Option<NativeAdapter>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NativeSignal {
    AdvertisedDbm(i16),
    BredrLinkMarginDb(i16),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NativeAdapter {
    pub name: String,
    pub address: Option<String>,
    pub vendor: Option<String>,
    pub model: Option<String>,
    pub hardware_id: Option<String>,
    pub path: Option<String>,
}

pub struct NativeButtonInput {
    pub index: usize,
    pub pressed: bool,
}

const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");

pub fn open_settings() -> Result<()> {
    qol_apps::desktop_integration::open_plugin_settings(PLUGIN_ID)
        .context("failed to open settings URL")
}
