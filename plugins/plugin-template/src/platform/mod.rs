use anyhow::{Context, Result};

const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");

pub struct PlatformSupport {
    pub name: &'static str,
    pub supported: bool,
}

pub fn open_settings() -> Result<()> {
    qol_apps::desktop_integration::open_plugin_settings(PLUGIN_ID)
        .context("failed to open settings URL")
}

pub fn current_support() -> PlatformSupport {
    let name = std::env::consts::OS;
    PlatformSupport {
        name,
        supported: matches!(name, "linux" | "macos"),
    }
}
