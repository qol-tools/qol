use anyhow::{Context, Result};

const PLUGIN_ID: &str = env!("QOL_PLUGIN_ID");

pub fn open_settings() -> Result<()> {
    let settings_url = qol_conventions::settings_url(PLUGIN_ID);
    open::that(&settings_url).context("failed to open settings URL")
}
